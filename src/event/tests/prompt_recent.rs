//! 最近页面抽屉事件测试：快捷键/按钮开关、键盘接管、单击跳转、悬停与滚轮。

use super::test_support::*;
use super::*;
use crate::event::session::Geometry;

fn key(state: &mut AppState, geo: &Geometry, code: KeyCode, modifiers: KeyModifiers) -> bool {
    let config = Config::default();
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(code, modifiers)),
        state,
        &mut sessions,
        geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    dirty
}

fn click(state: &mut AppState, geo: &Geometry, column: u16, row: u16) {
    let config = Config::default();
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), column, row),
        state,
        &mut sessions,
        geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
}

fn drawer_rects(state: &AppState, geo: &Geometry) -> (Rect, Rect) {
    let rows = ui::prompt_frame::recent_rows(state);
    let split = crate::layout::prompt_body_split(geo.view.prompt, true, rows.len());
    (
        split.recent_text.expect("drawer content"),
        split.recent_scrollbar.expect("drawer scrollbar gutter"),
    )
}

#[test]
fn ctrl_r_toggles_drawer_in_every_prompt_mode() {
    let config = Config::default();
    for mode in [PromptMode::Editor, PromptMode::Tree, PromptMode::Pages] {
        let mut state = paged_prompt_state();
        update::focus_prompt(&mut state);
        match mode {
            PromptMode::Editor => {}
            PromptMode::Tree => update::show_prompt_tree(&mut state),
            PromptMode::Pages => assert!(update::show_prompt_pages(&mut state)),
        }
        let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
        assert!(
            key(&mut state, &geo, KeyCode::Char('r'), KeyModifiers::CONTROL),
            "{mode:?} 按 Ctrl+R 需要重绘"
        );
        assert!(state.workspaces[0].prompt_panel.recent_open, "{mode:?}");
        key(&mut state, &geo, KeyCode::Char('r'), KeyModifiers::CONTROL);
        assert!(!state.workspaces[0].prompt_panel.recent_open, "{mode:?}");
        assert_eq!(
            state.workspaces[0].prompt_panel.mode, mode,
            "开关抽屉不切换视图"
        );
    }
}

#[test]
fn clicking_rec_button_toggles_drawer() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    update::focus_prompt(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    let rec = crate::layout::prompt_toolbar_button_rect(
        geo.view.prompt,
        crate::layout::PromptToolbarButton::Recent,
        PromptMode::Editor,
        Some("1/2"),
    )
    .expect("rec visible");
    click(&mut state, &geo, rec.x, rec.y);
    assert!(state.workspaces[0].prompt_panel.recent_open);
    click(&mut state, &geo, rec.x, rec.y);
    assert!(!state.workspaces[0].prompt_panel.recent_open);
}

#[test]
fn clicking_recent_entry_opens_page_and_keeps_drawer() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    update::focus_prompt(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    key(&mut state, &geo, KeyCode::Char('r'), KeyModifiers::CONTROL);
    let (area, _) = drawer_rects(&state, &geo);
    let rows = ui::prompt_frame::recent_rows(&state);
    let beta = rows
        .iter()
        .position(|row| row.page_id == crate::app::prompt::PromptPageId::Saved(70))
        .expect("beta row");
    click(&mut state, &geo, area.x + 2, area.y + beta as u16 * 2);

    let panel = &state.workspaces[0].prompt_panel;
    assert!(panel.recent_open, "跳转后抽屉保持展开");
    assert_eq!(panel.mode, PromptMode::Editor);
    assert_eq!(
        panel.editing,
        Some(crate::app::prompt::PromptEditing::Item(7))
    );
    assert_eq!(state.prompt.text(), "beta");
    assert_eq!(
        panel.tree.item(7).map(|item| item.active_page),
        Some(crate::app::prompt::PromptPageId::Saved(70))
    );
    assert_eq!(panel.recent_active, Some(beta), "点击后高亮跟随目标");
}

#[test]
fn clicking_recent_entry_opens_from_tree_and_pages() {
    let config = Config::default();
    for mode in [PromptMode::Tree, PromptMode::Pages] {
        let mut state = paged_prompt_state();
        update::focus_prompt(&mut state);
        match mode {
            PromptMode::Tree => update::show_prompt_tree(&mut state),
            PromptMode::Pages => assert!(update::show_prompt_pages(&mut state)),
            PromptMode::Editor => unreachable!(),
        }
        let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
        key(&mut state, &geo, KeyCode::Char('r'), KeyModifiers::CONTROL);
        let (area, _) = drawer_rects(&state, &geo);
        let rows = ui::prompt_frame::recent_rows(&state);
        let beta = rows
            .iter()
            .position(|row| row.page_id == crate::app::prompt::PromptPageId::Saved(70))
            .expect("beta row");
        click(&mut state, &geo, area.x + 2, area.y + beta as u16 * 2);
        assert_eq!(
            state.workspaces[0].prompt_panel.mode,
            PromptMode::Editor,
            "{mode:?} 点击后进入编辑视图"
        );
        assert_eq!(state.prompt.text(), "beta", "{mode:?}");
        assert!(state.workspaces[0].prompt_panel.recent_open, "{mode:?}");
    }
}

#[test]
fn recent_scrollbar_supports_track_click_and_thumb_drag() {
    use crate::app::prompt::{PromptItem, PromptPage};
    let config = Config::default();
    let mut state = demo_terminal();
    let mut item = PromptItem::single(7, None, "paged", "alpha");
    for index in 1..6 {
        item.pages.push(PromptPage::saved(
            70 + index,
            format!("Page {}", index + 1),
            format!("body {index}"),
        ));
    }
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    assert!(update::edit_prompt_item(&mut state, 0, 7));
    update::focus_prompt(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    key(&mut state, &geo, KeyCode::Char('r'), KeyModifiers::CONTROL);
    // 本用例只验证滚动条：把键盘选中与滚动归零，确保轨道点击从顶部开始。
    state.workspaces[0].prompt_panel.recent_active = None;
    state.workspaces[0].prompt_panel.recent_scroll = 0;
    let bar = ui::prompt_frame::prompt_recent_scrollbar(geo.view.prompt, &state)
        .expect("recent scrollbar");
    // 轨道点击：跳转到非零偏移。
    click(&mut state, &geo, bar.thumb.x, bar.track.bottom() - 1);
    assert!(state.workspaces[0].prompt_panel.recent_scroll > 0);
    assert!(
        state.prompt_recent_scroll_drag.is_none(),
        "轨道点击不进入拖拽"
    );

    // thumb 拖拽：按下进入拖拽，移动更新偏移，松开清空。
    let bar = ui::prompt_frame::prompt_recent_scrollbar(geo.view.prompt, &state)
        .expect("recent scrollbar");
    click(&mut state, &geo, bar.thumb.x, bar.thumb.y);
    assert_eq!(state.prompt_recent_scroll_drag, Some(0));
    let before = state.workspaces[0].prompt_panel.recent_scroll;
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            bar.thumb.x,
            bar.track.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.workspaces[0].prompt_panel.recent_scroll <= before);
    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            bar.thumb.x,
            bar.track.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt_recent_scroll_drag.is_none());
}

#[test]
fn keyboard_defaults_to_current_page_and_navigates_to_open() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    update::focus_prompt(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    key(&mut state, &geo, KeyCode::Char('r'), KeyModifiers::CONTROL);
    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(panel.recent_active, Some(1), "默认选中当前编辑页");

    key(&mut state, &geo, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(
        state.workspaces[0].prompt_panel.recent_active,
        Some(0),
        "↑ 移动到上一条"
    );
    key(&mut state, &geo, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(state.prompt.text(), "beta", "Enter 打开高亮记录");
    assert!(state.workspaces[0].prompt_panel.recent_open);
    assert_eq!(
        state.workspaces[0].prompt_panel.recent_active,
        Some(0),
        "打开后高亮跟随目标"
    );

    key(&mut state, &geo, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!state.workspaces[0].prompt_panel.recent_open);
    assert_eq!(state.workspaces[0].prompt_panel.recent_active, None);
    assert_eq!(
        state.workspaces[0].prompt_panel.mode,
        PromptMode::Editor,
        "Esc 回到原视图"
    );
    assert_eq!(state.prompt.text(), "beta", "收起抽屉不改正文");
}

#[test]
fn keyboard_takes_over_and_swallows_editor_keys() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    update::focus_prompt(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    let before = state.prompt.text().to_string();
    key(&mut state, &geo, KeyCode::Char('r'), KeyModifiers::CONTROL);
    for (code, modifiers) in [
        (KeyCode::Char('z'), KeyModifiers::NONE),
        (KeyCode::Char('l'), KeyModifiers::CONTROL),
        (KeyCode::Char('p'), KeyModifiers::CONTROL),
        (KeyCode::Char('s'), KeyModifiers::CONTROL),
    ] {
        key(&mut state, &geo, code, modifiers);
    }
    assert_eq!(state.prompt.text(), before, "抽屉展开时按键不写入正文");
    assert_eq!(
        state.workspaces[0].prompt_panel.mode,
        PromptMode::Editor,
        "视图切换快捷键同样被抽屉吞掉"
    );
}

#[test]
fn close_button_and_escape_restore_previous_view() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    update::focus_prompt(&mut state);
    update::show_prompt_tree(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    key(&mut state, &geo, KeyCode::Char('r'), KeyModifiers::CONTROL);
    let close = ui::prompt_frame::prompt_recent_close_rect(geo.view.prompt, &state)
        .expect("drawer close button");
    click(&mut state, &geo, close.x + 1, close.y);
    assert!(!state.workspaces[0].prompt_panel.recent_open);
    assert_eq!(
        state.workspaces[0].prompt_panel.mode,
        PromptMode::Tree,
        "关闭按钮回到 Tree 视图"
    );
    assert_eq!(state.workspaces[0].prompt_panel.recent_active, None);
}

#[test]
fn keyboard_navigation_keeps_selection_visible() {
    use crate::app::prompt::{PromptItem, PromptPage};
    let config = Config::default();
    let mut state = demo_terminal();
    let mut item = PromptItem::single(7, None, "paged", "alpha");
    for index in 1..6 {
        item.pages.push(PromptPage::saved(
            70 + index,
            format!("Page {}", index + 1),
            format!("body {index}"),
        ));
    }
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    assert!(update::edit_prompt_item(&mut state, 0, 7));
    update::focus_prompt(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    key(&mut state, &geo, KeyCode::Char('r'), KeyModifiers::CONTROL);
    let rows = ui::prompt_frame::recent_rows(&state);
    let visible =
        crate::layout::prompt_body_split(geo.view.prompt, true, rows.len()).visible_entries;
    for _ in 0..3 {
        key(&mut state, &geo, KeyCode::Up, KeyModifiers::NONE);
        let panel = &state.workspaces[0].prompt_panel;
        let active = panel.recent_active.expect("active recent");
        assert!(
            active >= panel.recent_scroll && active < panel.recent_scroll + visible,
            "选中项必须保持在可见窗口内：active={active} scroll={} visible={visible}",
            panel.recent_scroll
        );
    }
}

#[test]
fn hovering_recent_entry_sets_and_clears_highlight() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    update::focus_prompt(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    key(&mut state, &geo, KeyCode::Char('r'), KeyModifiers::CONTROL);
    let (area, _) = drawer_rects(&state, &geo);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Moved, area.x + 2, area.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt_recent_hover, Some(0));
    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            geo.view.prompt.x + 2,
            geo.view.prompt.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt_recent_hover, None, "离开抽屉即清除悬停");
}

#[test]
fn wheel_over_drawer_scrolls_entries_without_touching_editor() {
    use crate::app::prompt::{PromptItem, PromptPage};
    let config = Config::default();
    let mut state = demo_terminal();
    let mut item = PromptItem::single(7, None, "paged", "alpha");
    for index in 1..5 {
        item.pages.push(PromptPage::saved(
            70 + index,
            format!("Page {}", index + 1),
            format!("body {index}"),
        ));
    }
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    assert!(update::edit_prompt_item(&mut state, 0, 7));
    update::focus_prompt(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    key(&mut state, &geo, KeyCode::Char('r'), KeyModifiers::CONTROL);
    let (area, _) = drawer_rects(&state, &geo);
    let scroll_before = state.prompt.scroll();
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, area.x + 2, area.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.workspaces[0].prompt_panel.recent_scroll > 0);
    assert_eq!(state.prompt.scroll(), scroll_before, "不穿透滚动编辑器");
}
