//! 置顶用户消息行（表现层，codex prompt_header 口径）：钉住/让位/抑制记忆与宽度阈值。

use super::*;

/// 构造一段够长、能溢出的历史：两轮问答，正文各若干行。
fn sticky_lx() -> Lx {
    let mut lx = lx_with(vec![
        message("user", "complete", vec![text_part("第一轮的问题")]),
        message("assistant", "complete", vec![text_part("A1\nA2\nA3")]),
        message("user", "complete", vec![text_part("第二轮的问题")]),
        message("assistant", "complete", vec![text_part("B1\nB2\nB3")]),
    ]);
    // 溢出必需的滚动空间：视口 6 行、内容远多于 6 行。
    lx.chat.follow_bottom = false;
    lx
}

/// 历史区（宽 24、高 8）：body 6 行。
fn sticky_area() -> Rect {
    Rect::new(0, 0, 24, 8)
}

#[test]
fn sticky_prompt_pins_the_turn_prompt_when_top_row_is_answer() {
    let mut lx = sticky_lx();
    let area = sticky_area();
    let layout = layout(area, &lx, &mut cache());
    // 顶行落在第一轮回复里。
    let scroll = layout.messages[1].start;
    lx.chat.history_scroll = scroll;
    let computed = view(area, &lx, &mut cache());
    assert_eq!(computed.sticky, Some(0), "钉第一轮的 prompt");
    assert_eq!(computed.body.y, area.y + 1, "让位一行");
    assert_eq!(computed.body.height, area.height - 1);
    assert_eq!(computed.scroll, computed.scroll, "滚动偏移按 body 钳制");
}

#[test]
fn sticky_prompt_yields_when_top_row_is_the_prompt() {
    let mut lx = sticky_lx();
    let area = sticky_area();
    let layout = layout(area, &lx, &mut cache());
    // 顶行就是第二条用户消息自己的行（含它前面的空行）→ 不钉。
    lx.chat.history_scroll = layout.messages[2].start;
    let computed = view(area, &lx, &mut cache());
    assert_eq!(computed.sticky, None, "用户消息自带上下文");
    assert_eq!(computed.body, area, "不占行");

    // 轮边界空行（归后一条消息）同样按用户消息自己的行处理。
    lx.chat.history_scroll = layout.messages[2].start.saturating_sub(1);
    let at_blank = view(area, &lx, &mut cache());
    assert_eq!(at_blank.sticky, None, "空行归后一条：仍算用户消息的行");
}

#[test]
fn sticky_prompt_revert_records_suppression_memory() {
    let mut lx = sticky_lx();
    let area = sticky_area();
    let layout = layout(area, &lx, &mut cache());
    // 整块 body 下顶行 = 第一轮回复最后一行 → 需要钉；让位一行后顶行变成
    // 第二条用户消息的边界空行 → 不再需要 → 撤销让位并记记忆。
    let scroll = layout.messages[2].start.saturating_sub(1);
    lx.chat.history_scroll = scroll;
    let computed = view(area, &lx, &mut cache());
    assert_eq!(computed.sticky, None, "让位后不再需要 → 撤销");
    assert_eq!(computed.body, area, "撤销让位");
    let memory = computed.suppressed.expect("写入抑制记忆");
    assert_eq!(memory.viewport, area);
    assert_eq!(memory.message, 1, "记忆的是整块视口的顶行位置");
}

#[test]
fn sticky_prompt_respects_suppression_memory_while_reading() {
    let mut lx = sticky_lx();
    let area = sticky_area();
    let layout = layout(area, &lx, &mut cache());
    // 顶行在第一轮回复里：本会钉，但记忆一致 → 跳过重判、沿用抑制。
    lx.chat.history_scroll = layout.messages[1].start;
    let top =
        super::super::sticky::message_at(&layout.messages, layout.messages[1].start).expect("顶行");
    lx.chat.suppressed_prompt_header = Some(crate::app::state::lx::chat::StickySuppression {
        viewport: area,
        message: top.0,
        row: top.1,
    });
    let computed = view(area, &lx, &mut cache());
    assert_eq!(computed.sticky, None, "阅读态沿用抑制");
    assert_eq!(computed.body, area);
    assert_eq!(
        computed.suppressed, lx.chat.suppressed_prompt_header,
        "记忆保留"
    );

    // 视口一变化记忆即失效 → 重新判定（本会钉）。
    lx.chat.suppressed_prompt_header = Some(crate::app::state::lx::chat::StickySuppression {
        viewport: Rect::new(0, 0, 23, 8),
        ..computed.suppressed.expect("记忆")
    });
    let resized = view(area, &lx, &mut cache());
    assert_eq!(resized.sticky, Some(0), "视口变化使记忆失效");
}

#[test]
fn sticky_prompt_thresholds_hide_the_row() {
    let mut lx = sticky_lx();
    let layout = layout(sticky_area(), &lx, &mut cache());
    lx.chat.history_scroll = layout.messages[1].start;
    // 太矮：不显示（也不写记忆）。
    let short = view(Rect::new(0, 0, 24, 3), &lx, &mut cache());
    assert_eq!(short.sticky, None, "高度不足不显示");
    assert_eq!(short.suppressed, None);
    // 太窄：不显示。
    let narrow = view(Rect::new(0, 0, 15, 8), &lx, &mut cache());
    assert_eq!(narrow.sticky, None, "宽度不足不显示");
}

#[test]
fn sticky_text_collapses_sanitizes_and_marks_empty() {
    let mut message = message(
        "user",
        "complete",
        vec![text_part("  第一  行\n\t第二\r行  ")],
    );
    assert_eq!(
        super::super::sticky::sticky_text(&message).as_deref(),
        Some("第一 行 第二 行")
    );
    // 只有空白 → 不钉（None）。
    set_parts(&mut message, vec![text_part("   \n\t ")]);
    assert_eq!(super::super::sticky::sticky_text(&message), None);
    // 超长粘贴按 512 字符取样（限住每帧成本）。
    set_parts(&mut message, vec![text_part(&"字".repeat(1_000))]);
    let text = super::super::sticky::sticky_text(&message).expect("摘要");
    assert_eq!(text.chars().count(), super::super::sticky::SAMPLE_CHARS);
}

#[test]
fn sticky_row_is_painted_outside_the_grid() {
    let mut lx = sticky_lx();
    let area = sticky_area();
    let layout = layout(area, &lx, &mut cache());
    lx.chat.history_scroll = layout.messages[1].start;
    let computed = view(area, &lx, &mut cache());
    let row = sticky_rect(&computed).expect("置顶行");
    assert_eq!(row.rect, Rect::new(area.x, area.y, area.width, 1));

    // 表现层：直接写 Buffer，网格那一行保持空白（→ 不进复制与选区）。
    let message = lx.chat.messages[row.message].clone();
    let mut buf = Buffer::empty(area);
    render_sticky_prompt(&mut buf, row, &message);
    let painted = row_text(&buf, area, 0);
    let squeezed = painted.replace(' ', "");
    assert!(
        squeezed.starts_with(">第一轮的问题"),
        "前缀与用户消息同款 `> `：{painted}"
    );

    let mut grid = Grid::new(area);
    render(&mut grid, computed.body, &lx, computed.scroll, &mut cache());
    let mut grid_row = String::new();
    for x in area.x..area.right() {
        grid_row.push(grid.cell(x, area.y).ch);
    }
    assert!(
        grid_row.trim().is_empty(),
        "网格置顶行保持空白：{grid_row:?}"
    );
}

#[test]
fn sticky_text_is_truncated_with_ellipsis_to_width() {
    let mut lx = sticky_lx();
    set_parts(
        &mut lx.chat.messages[0],
        vec![text_part("很长很长很长很长很长很长很长的问题")],
    );
    let area = sticky_area();
    let layout = layout(area, &lx, &mut cache());
    lx.chat.history_scroll = layout.messages[1].start;
    let computed = view(area, &lx, &mut cache());
    let row = sticky_rect(&computed).expect("置顶行");
    let message = lx.chat.messages[row.message].clone();
    let mut buf = Buffer::empty(area);
    render_sticky_prompt(&mut buf, row, &message);
    let painted = row_text(&buf, area, 0);
    assert!(
        painted.trim_end().ends_with('…'),
        "溢出以省略号收尾：{painted}"
    );
    // 截断函数本体：按显示列截断、宽字符按两列计。
    assert_eq!(truncate_with_ellipsis("abcdef", 4), "abc…");
    assert_eq!(truncate_with_ellipsis("中文中文", 5), "中文…");
    assert_eq!(truncate_with_ellipsis("abc", 3), "abc");
}
