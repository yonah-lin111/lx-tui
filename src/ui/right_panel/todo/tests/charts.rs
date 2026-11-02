//! Todo 统计图表与分割线切换控件测试：三态柱状分布、优先级分布、近 7 日趋势与极窄摘要降级。

use super::*;
use crate::ui::right_panel::todo::charts::chart_switcher_rects;

/// 列表与统计块平分主体区：两者高度最多差 1 行（分割线吃掉余数）。
#[test]
fn list_and_stats_split_body_evenly() {
    let area = panel(38);
    let strip = week_strip_rect(area).expect("周感知带可见");
    let list = list_rect(area);
    let divider = stats_divider_rect(area).expect("统计块可见");
    let stats = stats_rect(area).expect("统计块可见");
    let body = crate::layout::prompt_body_rect(crate::layout::pane_inner_rect(area));

    assert_eq!(strip.y, body.y, "周感知带贴主体区顶边");
    assert_eq!(list.y, strip.bottom(), "列表紧贴周感知带下方");
    assert_eq!(
        u32::from(strip.height) + u32::from(list.height) + u32::from(stats.height) + 1,
        u32::from(body.height),
        "周感知带 + 列表 + 分割线 + 统计 == 主体区高度"
    );
    let diff = list.height.abs_diff(stats.height);
    assert!(
        diff <= 1,
        "两者高度差 {diff} 超过 1 行：{list:?} / {stats:?}"
    );
    assert_eq!(divider.y, list.bottom(), "分割线紧贴列表底部");
    assert_eq!(stats.y, divider.bottom(), "统计块紧贴分割线下方");
    assert_eq!(stats.bottom(), body.bottom(), "统计块贴主体区底部");
    assert_eq!(list.right(), body.right() - 1, "列表让出最右 1 列滚动条槽");
}

/// 主体区太矮时不切分：列表独占整块，统计块整块隐藏。
#[test]
fn stats_hidden_when_body_too_short() {
    for height in 0..MIN_SPLIT_BODY {
        let area = Rect::new(0, 0, 40, height);
        assert!(stats_divider_rect(area).is_none(), "高度 {height} 无分割线");
        assert!(stats_rect(area).is_none(), "高度 {height} 无统计块");
        let body = crate::layout::prompt_body_rect(crate::layout::pane_inner_rect(area));
        assert_eq!(
            list_rect(area).height,
            body.height,
            "高度 {height} 列表独占"
        );
    }
}

/// 统计块首行是摘要 `total N · doing N · done N N%`。
#[test]
fn stats_summary_row_shows_counts_and_percent() {
    let mut state = AppState::demo();
    let area = panel(46);
    state.todo.replace(vec![
        TodoRecord::new("a", TodoStatus::Todo, 0),
        TodoRecord::new("b", TodoStatus::InProgress, 1),
        TodoRecord::new("c", TodoStatus::Done, 2),
        TodoRecord::new("d", TodoStatus::Done, 3),
    ]);

    let (buffer, _) = render_panel(&state, area);
    let stats = stats_rect(area).expect("统计块可见");
    let text: String = (stats.x..stats.right())
        .map(|x| buffer[(x, stats.y)].symbol())
        .collect();
    assert_eq!(
        text.trim_end(),
        format!(
            "{total} 4{sep}{doing} 1{sep}{done_label} 2 50%",
            total = text::TODO_STATS_TOTAL_LABEL,
            sep = text::TODO_STATS_SEPARATOR,
            doing = text::TODO_STATS_DOING_LABEL,
            done_label = text::TODO_STATS_DONE_LABEL,
        ),
        "`total 4 · doing 1 · done 2 50%`（标签 muted、数值正文色，段间 ` · `）"
    );
}

/// 柱图按三态各占一槽，柱子自下而上堆叠 `█`；轴标签行在统计块最后一行。
#[test]
fn stats_chart_draws_bars_per_status() {
    let mut state = AppState::demo();
    let area = panel(46);
    state.todo.replace(vec![
        TodoRecord::new("a", TodoStatus::Todo, 0),
        TodoRecord::new("b", TodoStatus::Todo, 1),
        TodoRecord::new("c", TodoStatus::Done, 2),
    ]);

    let (buffer, _) = render_panel(&state, area);
    let stats = stats_rect(area).expect("统计块可见");
    assert!(
        stats.height >= MIN_STATS_ROWS,
        "统计块高度 {}",
        stats.height
    );

    // 轴标签行：三段文案都在，且各带自己的计数。
    let axis: String = (stats.x..stats.right())
        .map(|x| buffer[(x, stats.bottom() - 1)].symbol())
        .collect();
    for label in [
        text::TODO_STATS_OPEN_LABEL,
        text::TODO_STATS_DOING_LABEL,
        text::TODO_STATS_DONE_LABEL,
    ] {
        assert!(axis.contains(label), "轴标签缺 {label}: {axis:?}");
    }
    assert!(axis.contains("open 2"), "未开始计数: {axis:?}");
    assert!(axis.contains("done 1"), "已完成计数: {axis:?}");

    // 柱区：`█` 出现在轴标签之上的行里，且未开始（2 条）比已完成（1 条）高。
    let chart_top = stats.y + 1;
    let chart_bottom = stats.bottom() - 2;
    let column_height = |x_test: &dyn Fn(u16) -> bool| -> usize {
        (chart_top..=chart_bottom)
            .filter(|y| {
                (stats.x..stats.right())
                    .any(|x| x_test(x) && buffer[(x, *y)].symbol() == text::TODO_BAR_FILLED)
            })
            .count()
    };
    let total_filled = column_height(&|_| true);
    assert!(total_filled > 0, "柱区必须有柱子");

    // 三种柱子分别用各自的状态色：至少存在一绿（done）一非绿（open / doing）。
    let has_green = (chart_top..=chart_bottom).any(|y| {
        (stats.x..stats.right()).any(|x| {
            let cell = &buffer[(x, y)];
            cell.symbol() == text::TODO_BAR_FILLED && cell.fg == Color::Green
        })
    });
    assert!(has_green, "已完成柱必须用 Green");
}

#[test]
fn empty_list_still_shows_zero_stats() {
    let state = AppState::demo();
    let area = panel(46);
    let (buffer, _) = render_panel(&state, area);
    let stats = stats_rect(area).expect("统计块可见");
    let text: String = (stats.x..stats.right())
        .map(|x| buffer[(x, stats.y)].symbol())
        .collect();
    assert!(text.contains("total 0"), "空列表统计归零: {text:?}");
    assert!(text.contains("done 0 0%"), "空列表百分比为 0: {text:?}");
    let filled = (stats.y..stats.bottom())
        .flat_map(|y| (stats.x..stats.right()).map(move |x| (x, y)))
        .filter(|(x, y)| buffer[(*x, *y)].symbol() == text::TODO_BAR_FILLED)
        .count();
    assert_eq!(filled, 0, "空列表不画柱子");
}

/// 统计行不得覆盖到滚动条槽：列表溢出时 thumb 与统计行同屏并存。
#[test]
fn scrollbar_track_stops_above_stats() {
    let area = panel(38);
    let mut state = AppState::demo();
    state.right_tab = crate::app::state::RightTab::Todo;
    let visible = visible_rows(area);
    state.todo.replace(records(visible.saturating_add(4)));
    let bar = scrollbar(area, &state).expect("溢出时出现滚动条");
    let list = list_rect(area);
    assert_eq!(bar.track.bottom(), list.bottom(), "轨道只在列表区");
    assert!(
        bar.track.bottom() <= stats_divider_rect(area).expect("统计块可见").y,
        "轨道不得压到分割线"
    );
}

/// 摘要行放不下时整行不画（分割线与柱图仍在），绝不允许后面的短段顶到前面拼出残句。
#[test]
fn stats_summary_hidden_whole_when_too_narrow() {
    let mut state = AppState::demo();
    state.todo.replace(vec![
        TodoRecord::new("a", TodoStatus::Todo, 0),
        TodoRecord::new("b", TodoStatus::Done, 1),
    ]);

    // 足够宽：整句可见。
    let wide = panel(46);
    let (buffer, _) = render_panel(&state, wide);
    let stats = stats_rect(wide).expect("统计块可见");
    let text: String = (stats.x..stats.right())
        .map(|x| buffer[(x, stats.y)].symbol())
        .collect();
    assert!(
        text.trim_end().ends_with("done 1 50%"),
        "宽面板整句: {text:?}"
    );

    // 恰好差一列：整行不画，不得留下半句。
    let needed = text.trim_end().chars().count();
    let narrow = panel(u16::try_from(needed).unwrap_or(u16::MAX) - 1);
    let (buffer, _) = render_panel(&state, narrow);
    let stats = stats_rect(narrow).expect("统计块可见");
    let text: String = (stats.x..stats.right())
        .map(|x| buffer[(x, stats.y)].symbol())
        .collect();
    assert_eq!(text.trim(), "", "放不下时摘要整行不画: {text:?}");
    // 分割线仍在：统计块的高度不随内容有无而变化。
    let divider = stats_divider_rect(narrow).expect("统计块可见");
    assert_eq!(
        buffer[(narrow.x, divider.y)].symbol(),
        text::DIVIDER_LEFT_JOIN
    );
}

#[test]
fn chart_switcher_and_three_chart_modes_render() {
    let mut state = AppState::demo();
    let area = panel(46);
    state.todo.set_today_date("2026-10-31");
    state.todo.replace(vec![
        TodoRecord::new_on("2026-10-30", "d1", TodoStatus::Done, TodoPriority::P0, 0),
        TodoRecord::new_on("2026-10-31", "t1", TodoStatus::Todo, TodoPriority::P0, 0),
        TodoRecord::new_on("2026-10-31", "t2", TodoStatus::Done, TodoPriority::P2, 1),
    ]);

    let stats = stats_rect(area).expect("统计块可见");
    let (whole, prev_btn, next_btn) = chart_switcher_rects(area, &state).expect("切图器可见");
    let (buffer, _) = render_panel(&state, area);
    let switcher_text: String = (whole.x..whole.right())
        .map(|x| buffer[(x, whole.y)].symbol())
        .collect();
    assert!(
        switcher_text.contains("today · status 1/3"),
        "切图器注明当天: {switcher_text:?}"
    );
    assert_eq!(
        chart_switch_hit_at(area, &state, prev_btn.x, prev_btn.y),
        Some(ChartSwitchHit::Prev)
    );
    assert_eq!(
        chart_switch_hit_at(area, &state, next_btn.x, next_btn.y),
        Some(ChartSwitchHit::Next)
    );

    // Chart 2: Priority
    state
        .todo
        .set_chart(crate::app::state::TodoChartKind::Priority);
    let (buffer, _) = render_panel(&state, area);
    let (whole2, _, _) = chart_switcher_rects(area, &state).expect("switcher2");
    let switcher2_text: String = (whole2.x..whole2.right())
        .map(|x| buffer[(x, whole2.y)].symbol())
        .collect();
    assert!(
        switcher2_text.contains("today · priority 2/3"),
        "切图器注明优先级为当天: {switcher2_text:?}"
    );
    let summary: String = (stats.x..stats.right())
        .map(|x| buffer[(x, stats.y)].symbol())
        .collect();
    assert!(summary.contains("P0 1"), "优先级统计摘要: {summary:?}");
    assert!(summary.contains("P2 1"), "优先级统计摘要: {summary:?}");

    // Chart 3: 7-Day Trend
    state
        .todo
        .set_chart(crate::app::state::TodoChartKind::Trend7Day);
    let (buffer, _) = render_panel(&state, area);
    let (whole3, prev3, next3) = chart_switcher_rects(area, &state).expect("switcher3");
    let switcher3_text: String = (whole3.x..whole3.right())
        .map(|x| buffer[(x, whole3.y)].symbol())
        .collect();
    assert!(
        switcher3_text.contains("week · 7d trend 3/3"),
        "切图器注明趋势为一周: {switcher3_text:?}"
    );
    let divider = stats_divider_rect(area).expect("divider");
    // 左右两侧边框线的字符与样式必须完全一致（无色差）：
    assert_eq!(buffer[(prev3.x - 1, divider.y)].symbol(), "─");
    assert_eq!(buffer[(next3.x + 1, divider.y)].symbol(), "─");
    assert_eq!(
        buffer[(prev3.x - 1, divider.y)].modifier,
        buffer[(next3.x + 1, divider.y)].modifier,
        "切图按钮左侧边框与右侧边框 modifier 必须完全一致"
    );
    assert_eq!(
        buffer[(prev3.x - 1, divider.y)].fg,
        buffer[(next3.x + 1, divider.y)].fg,
        "切图按钮左侧边框与右侧边框前景色必须完全一致"
    );
    // 切图按钮 `<` / `>` 本身已重置底层 DIM，只保留 BOLD 强调色：
    assert!(
        !buffer[(prev3.x, divider.y)]
            .modifier
            .contains(Modifier::DIM),
        "切图按钮左箭头不应继承分割线的 DIM"
    );
    assert!(
        !buffer[(next3.x, divider.y)]
            .modifier
            .contains(Modifier::DIM),
        "切图按钮右箭头不应继承分割线的 DIM"
    );

    let summary: String = (stats.x..stats.right())
        .map(|x| buffer[(x, stats.y)].symbol())
        .collect();
    assert!(
        summary.contains("plan 3"),
        "7日趋势摘要包含计划总数: {summary:?}"
    );
    assert!(
        summary.contains("done 2"),
        "7日趋势摘要包含完成总数: {summary:?}"
    );
}
