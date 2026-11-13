//! BTW 列表/编辑态、搜索光标与 composer 几何回归。

use super::*;
use ratatui::layout::Rect;

#[test]
fn btw_list_mode_has_search_and_five_rows() {
    let mut lx = Lx::new();
    lx.task = Some(TaskPage {
        tab: TaskTab::Btw,
        ..Default::default()
    });
    let height = task_panel_height(&lx);
    assert_eq!(height, 8, "列表态固定 8 行");
    let panel = Rect::new(0, 0, 80, height);
    let search = search_rect(panel);
    let rows = list_rows_rect(panel, TaskTab::Btw);
    assert_eq!(search.height, 1);
    assert_eq!(rows.height, 5);
    assert_eq!(search.y + 1, rows.y);
}

#[test]
fn btw_edit_mode_uses_only_the_composer() {
    let mut lx = Lx::new();
    lx.task = Some(TaskPage {
        tab: TaskTab::Btw,
        ..Default::default()
    });
    lx.btw.ensure_thread("qa-1");
    lx.btw.editing = true;
    let height = task_panel_height(&lx);
    assert_eq!(height, 3, "一行草稿 = 上下边框 + 1 行正文");
    let panel = Rect::new(0, 0, 80, height);
    let composer = super::btw::composer_rect(panel, &lx).expect("composer");
    assert_eq!(composer, panel);
    let mut grid = super::super::super::grid::Grid::new(panel);
    assert!(super::btw::draw_edit(&mut grid, composer, &lx, true).is_some());
    let top: String = (panel.x..panel.right())
        .map(|x| grid.cell(x, panel.y).ch)
        .collect();
    assert!(top.contains("BTW"), "composer 左侧标记：{top:?}");
}

#[test]
fn btw_detail_loading_is_centered_in_the_conversation() {
    let mut lx = Lx::new();
    lx.task = Some(TaskPage {
        tab: TaskTab::Btw,
        ..Default::default()
    });
    lx.btw.ensure_thread("qa-1");
    let area = Rect::new(0, 0, 80, 24);
    let output = task_output_rect(area, &lx).expect("output");
    let mut grid = super::super::super::grid::Grid::new(output);
    let mut cache = crate::ui::main_content::lx::PaneCache::default();
    super::btw::draw_conversation(&mut grid, output, &lx, &mut cache);
    let text: String = (output.y..output.bottom())
        .map(|y| {
            (output.x..output.right())
                .map(|x| grid.cell(x, y).ch)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains(crate::ui::text::LX_BTW_THREAD_LOADING),
        "线程加载中显示与 Agents 同款 loading: {text:?}"
    );
}

#[test]
fn btw_scrollbar_and_jump_button_follow_the_active_thread() {
    let mut lx = Lx::new();
    lx.task = Some(TaskPage {
        tab: TaskTab::Btw,
        ..Default::default()
    });
    lx.btw.ensure_thread("qa-1");
    let thread = lx.btw.active_thread_mut().expect("thread");
    thread.loaded = true;
    thread.chat.messages.push(text_message(
        "m-1",
        "assistant",
        &"overflow line\n".repeat(40),
    ));
    thread.rows_dirty = true;
    thread.follow = false;
    thread.scroll = 0;

    let area = Rect::new(0, 0, 80, 24);
    let mut cache = crate::ui::main_content::lx::PaneCache::default();
    let bar = crate::ui::main_content::lx::task_btw_scrollbar(area, &lx, &mut cache)
        .expect("overflowing BTW thread should expose a scrollbar");
    assert!(bar.total > bar.visible);

    let page = lx.task.as_ref().expect("task");
    assert!(!page.is_at_bottom(&lx), "上翻后不应视为触底");
    assert!(
        super::super::super::chat_bar::visible(&lx),
        "应显示回到底部按钮"
    );

    lx.btw.active_thread_mut().expect("thread").follow = true;
    assert!(page.is_at_bottom(&lx), "follow 时应视为触底");
    assert!(
        !super::super::super::chat_bar::visible(&lx),
        "触底后回到底部按钮应消失"
    );
}

#[test]
fn btw_list_hit_maps_filtered_rows_and_search_row() {
    use crate::app::state::lx::btw::BtwQaSummary;

    let mut lx = Lx::new();
    lx.task = Some(TaskPage {
        tab: TaskTab::Btw,
        ..Default::default()
    });
    lx.btw.qa = vec![
        BtwQaSummary {
            qa_id: "qa-1".into(),
            title: "alpha".into(),
            has_thread: true,
            status: "idle".into(),
        },
        BtwQaSummary {
            qa_id: "qa-2".into(),
            title: "beta".into(),
            has_thread: false,
            status: "running".into(),
        },
    ];
    lx.btw.active = Some("qa-2".into());
    lx.btw.sync_selection_after_filter();
    let rect = Rect::new(0, 0, 70, 8);
    let rows = list_rows_rect(rect, TaskTab::Btw);
    assert_eq!(
        hit(rect, &lx, rect.x, rows.y),
        Some(TaskHit::Qa(0)),
        "第一行映射回 qa[0]"
    );
    assert_eq!(
        hit(rect, &lx, rect.x, search_rect(rect).y),
        Some(TaskHit::Search)
    );
}

#[test]
fn btw_search_no_match_keeps_the_active_thread_detail() {
    use crate::app::state::lx::btw::BtwQaSummary;

    let mut lx = Lx::new();
    lx.task = Some(TaskPage {
        tab: TaskTab::Btw,
        ..Default::default()
    });
    lx.btw.qa = vec![BtwQaSummary {
        qa_id: "qa-1".into(),
        title: "alpha".into(),
        has_thread: true,
        status: "idle".into(),
    }];
    lx.btw.active = Some("qa-1".into());
    lx.btw.sync_selection_after_filter();
    lx.btw.search.insert_str("zzz");
    lx.btw.sync_selection_after_filter();
    assert!(lx.btw.filtered_qa_indices().is_empty());
    assert_eq!(lx.btw.active.as_deref(), Some("qa-1"));
    assert_eq!(
        lx.btw.selected_index(),
        None,
        "列表无匹配，但详情仍保留稳定 active"
    );
}

#[test]
fn btw_list_hardware_cursor_stays_on_the_search_row() {
    let mut lx = Lx::new();
    lx.task = Some(TaskPage {
        tab: TaskTab::Btw,
        ..Default::default()
    });
    lx.btw.ensure_thread("qa-1");
    let area = Rect::new(0, 0, 80, 24);
    let rects = super::super::super::task_rects(area, &lx).expect("task rects");
    let search = search_rect(rects.list);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    let cursor = super::super::super::render(
        area,
        &mut buf,
        &lx,
        true,
        None,
        &mut crate::ui::main_content::lx::PaneCache::default(),
    )
    .expect("focused search row must own the hardware cursor");
    assert_eq!(
        cursor,
        (search.y.saturating_sub(area.y), search.x.saturating_add(2)),
        "列表态光标只能落在搜索行，不得出现在详情区"
    );
}

#[test]
fn list_scrollbar_is_not_hit_or_drawn_in_edit_mode() {
    let mut lx = Lx::new();
    lx.task = Some(TaskPage {
        tab: TaskTab::Btw,
        ..Default::default()
    });
    lx.btw.ensure_thread("qa-1");
    lx.btw.editing = true;
    lx.btw.qa = (0..12)
        .map(|index| crate::app::state::lx::btw::BtwQaSummary {
            qa_id: format!("qa-{index}"),
            title: format!("question {index}"),
            has_thread: false,
            status: "idle".into(),
        })
        .collect();
    lx.btw.active = Some("qa-1".into());
    lx.btw.sync_selection_after_filter();
    let area = Rect::new(0, 0, 80, 24);
    assert!(
        super::super::super::task_list_scrollbar(area, &lx).is_none(),
        "编辑态不得暴露隐藏列表的滚动条命中区"
    );
}

#[test]
fn btw_search_cursor_uses_text_start_not_the_rendered_end() {
    let mut lx = Lx::new();
    lx.task = Some(TaskPage {
        tab: TaskTab::Btw,
        ..Default::default()
    });
    lx.btw.search.insert_str("abc");
    lx.btw.search.set_cursor(3);
    let area = Rect::new(0, 0, 80, 24);
    let rects = super::super::super::task_rects(area, &lx).expect("task rects");
    let search = search_rect(rects.list);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    let cursor = super::super::super::render(
        area,
        &mut buf,
        &lx,
        true,
        None,
        &mut crate::ui::main_content::lx::PaneCache::default(),
    )
    .expect("search cursor");
    assert_eq!(cursor.0, search.y.saturating_sub(area.y));
    assert_eq!(
        cursor.1,
        search.x.saturating_add(2).saturating_add(3),
        "光标列以文本起点为基准，不能叠加完整可见文本宽度"
    );

    lx.btw.search.clear();
    lx.btw.search.insert_str(&"x".repeat(200));
    lx.btw.search.set_cursor(200);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    let cursor = super::super::super::render(
        area,
        &mut buf,
        &lx,
        true,
        None,
        &mut crate::ui::main_content::lx::PaneCache::default(),
    )
    .expect("scrolled search cursor");
    assert!(
        cursor.1 < search.right() && cursor.1 > search.x,
        "长搜索文本滚动后光标仍须落在搜索框内：{cursor:?}"
    );
}

#[test]
fn btw_edit_hardware_cursor_stays_inside_the_composer() {
    let mut lx = Lx::new();
    lx.task = Some(TaskPage {
        tab: TaskTab::Btw,
        ..Default::default()
    });
    lx.btw.ensure_thread("qa-1");
    lx.btw.editing = true;
    let area = Rect::new(0, 0, 80, 24);
    let rects = super::super::super::task_rects(area, &lx).expect("task rects");
    let mut buf = ratatui::buffer::Buffer::empty(area);
    let cursor = super::super::super::render(
        area,
        &mut buf,
        &lx,
        true,
        None,
        &mut crate::ui::main_content::lx::PaneCache::default(),
    )
    .expect("composer must own the hardware cursor");
    assert_eq!(
        cursor.0,
        rects.list.y.saturating_add(1).saturating_sub(area.y)
    );
    assert!(cursor.1 >= rects.list.x.saturating_add(2));
}
