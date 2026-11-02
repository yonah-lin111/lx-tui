//! Todo 日程组件测试：一周快速点击感知带（Week Strip）与历史未完成顺延横幅（Rollover Banner）。

use super::*;
use crate::ui::right_panel::todo::schedule::rollover_button_rects;

#[test]
fn week_strip_and_rollover_banner_hit_and_render() {
    let mut state = AppState::demo();
    let area = panel(46);
    state.todo.set_today_date("2026-10-31");
    state.todo.replace(vec![
        TodoRecord::new_on(
            "2026-10-30",
            "yesterday-open",
            TodoStatus::Todo,
            TodoPriority::P0,
            0,
        ),
        TodoRecord::new_on(
            "2026-10-31",
            "today-open",
            TodoStatus::Todo,
            TodoPriority::P1,
            0,
        ),
    ]);

    let (buffer, _) = render_panel(&state, area);
    let header = crate::layout::prompt_header_rect(area).expect("header");
    let divider_y = header.y + crate::layout::PROMPT_TOOLBAR_HEIGHT;
    let divider_text: String = (area.x..area.right())
        .map(|x| buffer[(x, divider_y)].symbol())
        .collect();
    assert!(
        divider_text.contains("2026/10/31"),
        "周感知带上方左侧边框包含 yyyy/mm/dd: {divider_text:?}"
    );
    assert!(
        divider_text.contains("26-01"),
        "周感知带上方右侧边框包含周跨度 26-01: {divider_text:?}"
    );

    let strip = week_strip_rect(area).expect("周感知带可见");
    let strip_text: String = (strip.x..strip.right())
        .map(|x| buffer[(x, strip.y)].symbol())
        .collect();
    assert!(
        strip_text.contains("Mon") && strip_text.contains("Sat"),
        "周感知带包含英文星期缩写: {strip_text:?}"
    );
    assert!(
        !strip_text.contains("30") && !strip_text.contains("31"),
        "周感知带已移除日期号数字: {strip_text:?}"
    );
    assert_eq!(
        week_strip_hit_at(area, &state, strip.x, strip.y),
        Some(WeekStripHit::PrevWeek)
    );
    assert_eq!(
        week_strip_hit_at(area, &state, strip.right() - 1, strip.y),
        Some(WeekStripHit::NextWeek)
    );

    let banner = rollover_banner_rect(area, &state).expect("顺延横幅可见");
    let banner_text: String = (banner.x..banner.right())
        .map(|x| buffer[(x, banner.y)].symbol())
        .collect();
    assert!(
        banner_text.contains("past"),
        "横幅提示历史未完成: {banner_text:?}"
    );
    let (action_rect, dismiss_rect) = rollover_button_rects(area, &state).expect("横幅按钮可见");
    assert_eq!(
        rollover_hit_at(area, &state, action_rect.x, action_rect.y),
        Some(RolloverHit::MoveToToday)
    );
    assert_eq!(
        rollover_hit_at(area, &state, dismiss_rect.x, dismiss_rect.y),
        Some(RolloverHit::Dismiss)
    );
}
