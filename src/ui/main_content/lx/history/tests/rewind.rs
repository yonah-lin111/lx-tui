//! 独立撤销提示：与消息共享行预算、连续合并、选中底色。

use super::*;

fn rewind_notice(sequence: i64) -> crate::app::state::RewindNoticeView {
    crate::app::state::RewindNoticeView {
        id: format!("notice-{sequence}"),
        sequence,
        kind: "revert_and_delete".into(),
        restored_file_count: 2,
        files_already_restored: false,
    }
}

#[test]
fn tip_only_history_stays_on_history_view() {
    let mut lx = Lx::new();
    lx.chat.notices = vec![rewind_notice(0)];
    assert!(has_history(&lx), "messages 为空但提示非空仍是历史");
    let area = Rect::new(0, 0, 40, 10);
    let view = view(area, &lx, &mut cache());
    assert!(view.total_rows > 0);
}

#[test]
fn notices_and_messages_share_one_row_budget() {
    let mut lx = lx_with(vec![
        message("user", "complete", vec![text_part("hello")]),
        message("assistant", "complete", vec![text_part("world")]),
    ]);
    lx.chat.messages[0].sequence = 0;
    lx.chat.messages[1].sequence = 2;
    lx.chat.notices = vec![rewind_notice(1)];
    let area = Rect::new(0, 0, 40, 20);
    let layout = layout(area, &lx, &mut cache());
    let mut grid = Grid::new(area);
    let drawn = render(&mut grid, area, &lx, 0, &mut cache());
    assert_eq!(drawn, layout.total_rows, "布局与渲染行数必须一致");
    assert!(layout.total_rows > 4, "提示占两行，不能与消息共享同一行");
}

#[test]
fn selected_latest_user_message_uses_distinct_background() {
    let mut lx = lx_with(vec![message("user", "complete", vec![text_part("hello")])]);
    lx.rewind.selected = Some(crate::app::state::RewindTarget {
        session_id: "s".into(),
        user_message_id: "user-1".into(),
        expected_revision: 1,
        pane: crate::layout::PaneId::alloc(),
        input_at_open: String::new(),
        user_text: "hello".into(),
        kind: crate::app::state::RewindTargetKind::Turn,
    });
    let area = Rect::new(0, 0, 40, 10);
    let view = view(area, &lx, &mut cache());
    let mut grid = Grid::new(area);
    render(&mut grid, view.body, &lx, view.scroll, &mut cache());
    let highlighted = (view.body.y..view.body.bottom()).any(|y| {
        (view.body.x..view.body.right())
            .any(|x| grid.cell(x, y).style.bg == Some(ratatui::style::Color::Indexed(24)))
    });
    assert!(highlighted, "最新用户消息必须有选中底色");
}

#[test]
fn consecutive_rewind_notices_merge_with_count() {
    let mut lx = Lx::new();
    lx.chat.notices = vec![rewind_notice(0), rewind_notice(1), rewind_notice(2)];
    let area = Rect::new(0, 0, 60, 10);
    let layout = layout(area, &lx, &mut cache());
    let mut grid = Grid::new(area);
    let drawn = render(&mut grid, area, &lx, 0, &mut cache());
    assert_eq!(drawn, layout.total_rows);
    let text: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid.cell(x, y).ch)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("-- Rewind x3 --"),
        "连续提示合并为 xN：{text}"
    );
    assert_eq!(
        text.matches("-- Rewind").count(),
        1,
        "三条连续提示只画一条标题"
    );
    assert_eq!(layout.total_rows, 3, "合并后只占标题+描述+空行");
}

#[test]
fn notices_separated_by_messages_are_not_merged() {
    let mut lx = lx_with(vec![message(
        "assistant",
        "complete",
        vec![text_part("middle")],
    )]);
    lx.chat.messages[0].sequence = 1;
    lx.chat.notices = vec![rewind_notice(0), rewind_notice(2)];
    let area = Rect::new(0, 0, 60, 12);
    let mut grid = Grid::new(area);
    render(&mut grid, area, &lx, 0, &mut cache());
    let text: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| grid.cell(x, y).ch)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        text.matches("-- Rewind").count(),
        2,
        "被消息分隔的提示各自显示：{text}"
    );
}

#[test]
fn center_target_puts_user_message_middle_of_viewport() {
    let mut messages = Vec::new();
    for index in 0..24 {
        let role = if index % 2 == 0 { "user" } else { "assistant" };
        let mut message = message(role, "complete", vec![text_part("some history line")]);
        message.id = format!("m{index}");
        message.sequence = index;
        messages.push(message);
    }
    let lx = lx_with(messages);
    let area = Rect::new(0, 0, 40, 14);
    let (top, max_top) = history_center_target(area, &lx, "m10", &mut cache()).expect("scrollable");
    assert!(top > 0 && top < max_top, "目标应有上下可滚动空间");
    let rect = history_rect(area, &lx).expect("history");
    let layout = layout(rect, &lx, &mut cache());
    let rows = layout.messages[10];
    let height = usize::from(rect.height);
    assert!(
        rows.start >= top && rows.start < top + height,
        "目标必须在视口内"
    );
    assert!(
        rows.start.saturating_sub(top) <= height / 2 + 1,
        "目标应落在视口中部而不是底部"
    );
}

#[test]
fn center_target_is_none_when_content_fits() {
    let lx = lx_with(vec![message("user", "complete", vec![text_part("short")])]);
    assert_eq!(
        history_center_target(Rect::new(0, 0, 40, 20), &lx, "user-1", &mut cache()),
        None
    );
}
