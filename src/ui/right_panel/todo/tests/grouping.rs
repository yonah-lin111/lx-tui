use super::*;

#[test]
fn two_and_three_item_groups_render_shared_status_and_tree_connectors() {
    let mut state = AppState::demo();
    let area = Rect::new(0, 0, 46, 18);
    let today = state.todo.today_date().to_string();
    state.todo.replace(vec![
        TodoRecord::new_on(&today, "login api", TodoStatus::Todo, TodoPriority::P1, 0)
            .with_group_id(Some("g2".into())),
        TodoRecord::new_on(&today, "login test", TodoStatus::Todo, TodoPriority::P2, 1)
            .with_group_id(Some("g2".into())),
        TodoRecord::new_on(
            &today,
            "schema",
            TodoStatus::InProgress,
            TodoPriority::P0,
            2,
        )
        .with_group_id(Some("g3".into())),
        TodoRecord::new_on(
            &today,
            "migrate",
            TodoStatus::InProgress,
            TodoPriority::P1,
            3,
        )
        .with_group_id(Some("g3".into())),
        TodoRecord::new_on(&today, "docs", TodoStatus::InProgress, TodoPriority::P2, 4)
            .with_group_id(Some("g3".into())),
    ]);

    let (buffer, _) = render_panel(&state, area);
    let list = list_rect(area);

    // 2 条聚合组（g2）：anchor = (2 - 1) / 2 = 0，首行画 `[ ]-┌─ P1 `，次行画 `    └─ P2 `
    let row0 = row_text(&buffer, list, list.y);
    let row1 = row_text(&buffer, list, list.y + 1);
    assert!(
        row0.starts_with("❯ [ ]-┌─ P1 login api"),
        "2 条组首行应带共享状态框与 -┌─: {row0:?}"
    );
    assert!(
        row1.starts_with("      └─ P2 login test"),
        "2 条组末行状态槽应留空并画 └─: {row1:?}"
    );

    // 3 条聚合组（g3）：anchor = (3 - 1) / 2 = 1，中行画 `[~]-├─ P1 `
    let row2 = row_text(&buffer, list, list.y + 2);
    let row3 = row_text(&buffer, list, list.y + 3);
    let row4 = row_text(&buffer, list, list.y + 4);
    assert!(
        row2.starts_with("      ┌─ P0 schema"),
        "3 条组首行应画 ┌─: {row2:?}"
    );
    assert!(
        row3.starts_with("  [~]-├─ P1 migrate"),
        "3 条组中行应带共享状态框与 -├─: {row3:?}"
    );
    assert!(
        row4.starts_with("      └─ P2 docs"),
        "3 条组末行应画 └─: {row4:?}"
    );

    // 已完成聚合组的连接符使用 muted 但不叠加 CROSSED_OUT 删除线，保证框线清晰。
    state.todo.replace(vec![
        TodoRecord::new_on(&today, "done a", TodoStatus::Done, TodoPriority::P1, 0)
            .with_group_id(Some("gd".into())),
        TodoRecord::new_on(&today, "done b", TodoStatus::Done, TodoPriority::P1, 1)
            .with_group_id(Some("gd".into())),
    ]);
    let (done_buf, _) = render_panel(&state, area);
    let conn_cell = &done_buf[(row_branch_x(list) + 1, list.y)];
    assert_eq!(conn_cell.symbol(), "┌");
    assert!(conn_cell.modifier.contains(Modifier::DIM));
    assert!(
        !conn_cell.modifier.contains(Modifier::CROSSED_OUT),
        "连接符不得带删除线: {conn_cell:?}"
    );
}

#[test]
fn grouped_wrapped_item_draws_vertical_trunk_on_non_last_and_blank_on_last() {
    let mut state = AppState::demo();
    // 面板高度设高以便容纳多条 2 行折行子项。
    let area = Rect::new(0, 0, 40, 20);
    let list = list_rect(area);
    let grouped_first_w = text_width_for(list, true);
    let today = state.todo.today_date().to_string();
    let long_first = format!("{}tail1", "a".repeat(grouped_first_w));
    let long_last = format!("{}tail2", "b".repeat(grouped_first_w));
    state.todo.replace(vec![
        TodoRecord::new_on(&today, long_first, TodoStatus::Todo, TodoPriority::P1, 0)
            .with_group_id(Some("wrap-g".into())),
        TodoRecord::new_on(&today, long_last, TodoStatus::Todo, TodoPriority::P2, 1)
            .with_group_id(Some("wrap-g".into())),
    ]);

    let layout = layout_visible_items(area, &state);
    assert_eq!(layout.items[0].rendered_rows, 2, "非末项折为 2 行");
    assert_eq!(layout.items[1].rendered_rows, 2, "末项折为 2 行");

    let (buffer, _) = render_panel(&state, area);
    // 非末项（y = list.y）续行（y = list.y + 1）：连接符槽画 " │  "，正文从 list.x + 9 起
    let row0_cont = row_text(&buffer, list, list.y + 1);
    assert!(
        row0_cont.starts_with("      │  tail1"),
        "非末项折行续行必须延续竖线 │ 并对齐本条 P1 列: {row0_cont:?}"
    );

    // 末项（y = list.y + 2）续行（y = list.y + 3）：连接符槽画 4 列空格，正文从 list.x + 9 起
    let row1_cont = row_text(&buffer, list, list.y + 3);
    assert!(
        row1_cont.starts_with("         tail2"),
        "末项折行续行连接符槽必须留空并对齐本条 P2 列: {row1_cont:?}"
    );
}

#[test]
fn mixed_single_and_grouped_items_align_status_and_hit_priority_badges_accurately() {
    let mut state = AppState::demo();
    let area = panel(44);
    let today = state.todo.today_date().to_string();
    state.todo.replace(vec![
        TodoRecord::new_on(&today, "single", TodoStatus::Todo, TodoPriority::P0, 0),
        TodoRecord::new_on(&today, "grp-0", TodoStatus::Todo, TodoPriority::P1, 1)
            .with_group_id(Some("g".into())),
        TodoRecord::new_on(&today, "grp-1", TodoStatus::Todo, TodoPriority::P2, 2)
            .with_group_id(Some("g".into())),
        TodoRecord::new_on(&today, "grp-2", TodoStatus::Todo, TodoPriority::P3, 3)
            .with_group_id(Some("g".into())),
    ]);

    let list = list_rect(area);
    let sx = row_status_x(list);
    let single_px = row_priority_x_for(list, false);
    let group_px = row_priority_x_for(list, true);
    assert_eq!(sx, list.x + 2);
    assert_eq!(single_px, list.x + 6);
    assert_eq!(group_px, list.x + 9);

    // 状态框命中：单条（row 0）与 3 条组的锚点（row 2，offset = 1）命中，非锚点（row 1, row 3）不命中。
    assert_eq!(status_at(area, &state, sx, list.y), Some(0));
    assert_eq!(status_at(area, &state, sx + 2, list.y), Some(0));
    assert_eq!(status_at(area, &state, list.x + 1, list.y), None);
    assert_eq!(status_at(area, &state, sx, list.y + 1), None);
    assert_eq!(status_at(area, &state, sx, list.y + 2), Some(2));
    assert_eq!(status_at(area, &state, sx, list.y + 3), None);

    // 优先级徽标命中：单条命中 list.x + 6..8，组内子项命中 list.x + 9..11。
    assert_eq!(priority_at(area, &state, single_px, list.y), Some(0));
    assert_eq!(priority_at(area, &state, group_px, list.y), None);
    for (display, y_off) in [(1usize, 1u16), (2, 2), (3, 3)] {
        assert_eq!(
            priority_at(area, &state, single_px, list.y + y_off),
            None,
            "组内子项在单条优先级列（+6..+8）是连接符，不应误命中优先级"
        );
        assert_eq!(
            priority_at(area, &state, group_px, list.y + y_off),
            Some(display),
            "组内子项在 +9..+11 列必须准确命中优先级"
        );
    }
}

#[test]
fn dragging_source_row_suppresses_hover_selection_background() {
    let mut state = AppState::demo();
    let area = panel(44);
    let today = state.todo.today_date().to_string();
    let item0 = TodoRecord::new_on(&today, "source row", TodoStatus::Todo, TodoPriority::P1, 0);
    let item0_id = item0.id.clone();
    let item1 = TodoRecord::new_on(&today, "target row", TodoStatus::Todo, TodoPriority::P2, 1);
    state.todo.replace(vec![item0, item1]);
    state.right_tab = RightTab::Todo;
    state.set_focus(crate::app::state::FocusedContainer::Todo);
    state.todo_hover = Some(0);
    state.todo_drag = Some(crate::app::state::TodoDrag {
        source_id: item0_id,
        from_display: 0,
        pressed_at: std::time::Instant::now(),
        dragging: true,
        target_index: Some(1),
    });

    let (buffer, _) = render_panel(&state, area);
    let list = list_rect(area);
    let sel_bg = style::selection().bg;
    let row0_cell = &buffer[(list.x + 9, list.y)];
    let row1_cell = &buffer[(list.x + 9, list.y + 1)];
    assert!(
        row0_cell.modifier.contains(Modifier::DIM),
        "被拖拽源行应置灰: {row0_cell:?}"
    );
    assert_ne!(
        row0_cell.bg,
        sel_bg.unwrap_or(Color::Reset),
        "被拖拽源行即使残留 todo_hover 也不得铺选中底色: {row0_cell:?}"
    );
    assert_eq!(
        Some(row1_cell.bg),
        sel_bg,
        "被悬停的目标合并行应铺选中高亮底色: {row1_cell:?}"
    );

    // 状态锚点处于行内编辑态或组内新建幽灵行时，status_at 依然准确命中该锚点行。
    state.todo_drag = None;
    state.todo_hover = None;
    assert!(crate::app::update::begin_todo_edit(
        &mut state,
        crate::app::state::TodoEditTarget::Current(0)
    ));
    let sx = row_status_x(list);
    assert_eq!(
        status_at(area, &state, sx, list.y),
        Some(0),
        "正在编辑的状态锚点行应可命中状态框"
    );
    assert!(crate::app::update::cancel_todo_edit(&mut state));

    // 2 条聚合组首项后打开组内新建幽灵行：显示序列为 [0, ghost(1), 2]，状态锚点在 display = 1（幽灵行）。
    state.todo.replace(vec![
        TodoRecord::new_on(&today, "grp-a", TodoStatus::Todo, TodoPriority::P1, 0)
            .with_group_id(Some("g".into())),
        TodoRecord::new_on(&today, "grp-b", TodoStatus::Todo, TodoPriority::P1, 1)
            .with_group_id(Some("g".into())),
    ]);
    assert!(crate::app::update::begin_todo_edit(
        &mut state,
        crate::app::state::TodoEditTarget::After(0)
    ));
    assert_eq!(status_at(area, &state, sx, list.y), None);
    assert_eq!(
        status_at(area, &state, sx, list.y + 1),
        Some(1),
        "组内幽灵行作为 3 行聚合组的状态锚点时必须可命中状态框"
    );
    assert_eq!(status_at(area, &state, sx, list.y + 2), None);
}
