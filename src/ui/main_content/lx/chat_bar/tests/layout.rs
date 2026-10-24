//! 统计优先布局、极窄降级、宽字符与不安全文本（A12–A17）。

use super::*;

// ===== 统计优先布局（A12–A14）=====

/// A12：长中文 todo + 完整统计 → 文字截断、统计完整、行高不变。
#[test]
fn long_todo_keeps_the_full_stats() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    lx.chat.plan = Some(plan(&[(
        "修复输入框布局与中文截断",
        PlanStepView::InProgress,
    )]));
    with_stats(&mut lx);
    let (grid, bar) = draw_bar(&lx, 40);
    let row = row_text(&grid, bar);
    assert!(row.contains(STATS), "完整统计必须在：{row:?}");
    assert!(row.contains('…'), "长 todo 应截断：{row:?}");
    // 统计紧贴可见文字尾部一格，且文字没有侵入统计预算。
    let stats_col = col_of(&row, STATS);
    assert_eq!(char_before(&row, STATS), Some(' '), "{row:?}");
    let text_end = stats_col - 1;
    assert!(text_end > 1, "应保留可见文字：{row:?}");
    // 文字预算 = 可用宽度（栏宽 − 左右空档）− 完整统计 − 一格间隔。
    assert_eq!(
        text_end,
        bar.width - 2 - u16::try_from(STATS.width()).unwrap_or(0) - STATS_GAP,
        "文字不得吃掉统计预算：{row:?}"
    );
}

/// A12 续：512 字符的中英混合 todo 也只截断文字，统计不变、不换行。
#[test]
fn very_long_mixed_todo_truncates_and_keeps_stats() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    let long: String = (0..512)
        .map(|index| if index % 3 == 0 { '修' } else { 'a' })
        .collect();
    lx.chat.plan = Some(plan(&[(long.as_str(), PlanStepView::InProgress)]));
    with_stats(&mut lx);
    for width in [24u16, 40, 80] {
        let (grid, bar) = draw_bar(&lx, width);
        let row = row_text(&grid, bar);
        assert_eq!(bar.height, HEIGHT, "行高不变");
        assert!(row.contains(STATS), "width={width}: {row:?}");
        assert!(row.contains('…'), "width={width}: {row:?}");
        // 文案与统计之间只有一格间隔。
        let stats_col = col_of(&row, STATS);
        assert_eq!(
            char_before(&row, STATS),
            Some(' '),
            "width={width}: {row:?}"
        );
        assert!(
            !row[..row.find(STATS).unwrap_or(0)]
                .trim_end()
                .contains("  "),
            "width={width}: 文字内部不得有多余空格：{row:?}"
        );
        assert!(stats_col < bar.width, "width={width}: 统计必须在栏内");
    }
}

/// A13：宽度只够统计 → 不画文字、完整统计保留、不留无意义间隔。
#[test]
fn width_for_stats_only_hides_the_text() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    lx.chat.plan = Some(plan(&[("很长的中文任务描述", PlanStepView::InProgress)]));
    with_stats(&mut lx);
    let stats_w = u16::try_from(STATS.width()).unwrap_or(0);
    // bar 宽 = 1(左空档) + S + 1(右空档)。
    let (grid, bar) = draw_bar(&lx, stats_w + 2);
    let row = row_text(&grid, bar);
    assert_eq!(row, format!(" {STATS} "), "只画统计：{row:?}");
    assert!(!row.contains('…'), "文字整体隐藏时不留省略号：{row:?}");
}

/// A13 续：文字预算为 0 但统计更窄时，统计紧贴左侧起点（不右对齐）。
#[test]
fn hidden_text_leaves_no_leading_gap() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    lx.chat.plan = Some(plan(&[("任务", PlanStepView::InProgress)]));
    lx.chat.turn_elapsed_ms = Some(0);
    // 只有 ↑token 时统计更窄：`(↓1 tok)` = 9 列。
    let (grid, bar) = draw_bar(&lx, 11);
    let row = row_text(&grid, bar);
    assert!(row.starts_with(" (↓1 tok)"), "{row:?}");
}

/// A14：W<S 时普通运行只画统计的可见前缀，不越界、不 panic、不整段丢。
#[test]
fn narrower_than_stats_clips_stats_safely() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    lx.chat.plan = Some(plan(&[("任务", PlanStepView::InProgress)]));
    with_stats(&mut lx);
    for width in 0u16..=16 {
        let (grid, bar) = draw_bar(&lx, width);
        let row = row_text(&grid, bar);
        assert!(row.width() <= usize::from(width), "width={width}: {row:?}");
        // 有可用列时必须给出统计前缀（不是整段主动隐藏）。
        if width >= 3 {
            assert!(row.contains('('), "width={width}: {row:?}");
        }
        assert!(
            !row.contains(STATS),
            "width={width} 放不下完整统计：{row:?}"
        );
    }
}

/// A14 续：W 恰好等于 S 时统计完整、文字整体隐藏。
#[test]
fn width_equal_to_stats_shows_full_stats_only() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    with_stats(&mut lx);
    let stats_w = u16::try_from(STATS.width()).unwrap_or(0);
    for extra in [0u16, 1] {
        let (grid, bar) = draw_bar(&lx, stats_w + 2 + extra);
        let row = row_text(&grid, bar);
        assert!(row.contains(STATS), "extra={extra}: {row:?}");
        assert!(row.trim().starts_with('('), "extra={extra}: {row:?}");
    }
}

// ===== 宽字符与不安全文本（A15–A17）=====

/// A15：文字只剩一列时遇汉字 → 不半写汉字、不污染统计或外侧保留列。
#[test]
fn wide_character_never_splits_at_the_stats_boundary() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    // 偶数 + 奇数宽度都试：文字预算可能是 1 列（只能画 `…`）。
    lx.chat.plan = Some(plan(&[("汉字汉字汉字", PlanStepView::InProgress)]));
    with_stats(&mut lx);
    for width in 18u16..=30 {
        let (grid, bar) = draw_bar(&lx, width);
        let row = row_text(&grid, bar);
        let stats_col = col_of(&row, STATS);
        // 统计前一列必须是空格（不能是汉字或半写占位）。
        assert_eq!(
            char_before(&row, STATS),
            Some(' '),
            "width={width}: {row:?}"
        );
        // 不得出现半写占位或把汉字挤进统计区。
        assert!(!row.contains('\0'), "width={width}: {row:?}");
        assert_eq!(
            char_at_col(&grid, bar, stats_col - 1),
            Some(' '),
            "width={width}: 统计左边一列必须是空格"
        );
    }
}

/// A15 续：预算 1 列时只画 `…`（不画半个汉字）。
#[test]
fn single_column_budget_draws_only_the_ellipsis() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    lx.chat.plan = Some(plan(&[("中文任务", PlanStepView::InProgress)]));
    with_stats(&mut lx);
    let stats_w = u16::try_from(STATS.width()).unwrap_or(0);
    // 可用宽度 = 1 + S + 1（G=1）→ 文字预算 1 列。
    let (grid, bar) = draw_bar(&lx, stats_w + 2 + 1 + 1);
    let row = row_text(&grid, bar);
    assert!(row.trim_start().starts_with('…'), "{row:?}");
    assert!(row.contains(STATS), "{row:?}");
}

/// A16：todo 含换行 / Tab / CR / ESC / 控制字符时始终单行、无终端转义。
#[test]
fn unsafe_todo_text_stays_single_line() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    lx.chat.plan = Some(plan(&[(
        "修复\n输入框\t布局\r\u{1b}[31m",
        PlanStepView::InProgress,
    )]));
    let (grid, bar) = draw_bar(&lx, 60);
    let row = row_text(&grid, bar);
    assert!(row.contains("修复 输入框 布局 [31m"), "{row:?}");
    assert!(!row.contains('\n') && !row.contains('\t') && !row.contains('\r'));
    assert!(!row.contains('\u{1b}'), "不得写入终端转义：{row:?}");
    // 原计划未被改写。
    let original = &lx.chat.plan.as_ref().expect("plan").steps[0].0;
    assert_eq!(original, "修复\n输入框\t布局\r\u{1b}[31m");
}

/// A17：净化后为空回退 Working（不留空白动画）；零宽字符不占列。
#[test]
fn blank_or_zero_width_todo_falls_back_to_working() {
    let mut lx = streaming_lx("a1", vec![text_step("答")]);
    lx.chat.plan = Some(plan(&[(" \n\t ", PlanStepView::InProgress)]));
    let (grid, bar) = draw_bar(&lx, 40);
    assert!(row_text(&grid, bar).contains(text::chat::LOADING_LABEL));

    // 可见字符之间夹零宽字符：可正常绘制，且不额外推进列。
    lx.chat.plan = Some(plan(&[("修复\u{200b}布局", PlanStepView::InProgress)]));
    let (grid, bar) = draw_bar(&lx, 40);
    let row = row_text(&grid, bar);
    assert!(row.contains("修复布局"), "零宽字符不占列：{row:?}");
}
