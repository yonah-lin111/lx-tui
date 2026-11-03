//! 最近页面抽屉行为测试：开关、滚动钳制与单击跳转。

use super::*;
use crate::app::prompt::{PromptEditing, PromptPage, PromptPageId};
use crate::app::state::FocusedContainer;
use crate::app::update::apply_prompt_tree;
use crate::db::{PromptPageRecord, PromptRecord};
use ratatui::layout::Rect;

fn demo_state() -> AppState {
    let mut state = AppState::demo();
    let prompts = vec![
        PromptRecord {
            id: 10,
            folder_id: None,
            name: "Alpha".into(),
            content: "alpha".into(),
            active_page_id: Some(10),
            pages: vec![PromptPageRecord {
                id: 10,
                position: 0,
                name: "Page 1".into(),
                content: "alpha".into(),
                updated_at: 100,
            }],
        },
        PromptRecord {
            id: 12,
            folder_id: None,
            name: "Beta".into(),
            content: "beta".into(),
            active_page_id: Some(12),
            pages: vec![PromptPageRecord {
                id: 12,
                position: 0,
                name: "Page 1".into(),
                content: "beta".into(),
                updated_at: 200,
            }],
        },
    ];
    apply_prompt_tree(
        &mut state,
        0,
        std::path::Path::new("/ws"),
        Vec::new(),
        prompts,
    );
    state.prompt.resize(30, 10);
    state
}

#[test]
fn toggle_opens_and_resets_scroll() {
    let mut state = demo_state();
    state.workspaces[0].prompt_panel.recent_scroll = 5;
    assert!(toggle_prompt_recent(&mut state));
    assert!(state.workspaces[0].prompt_panel.recent_open);
    assert_eq!(state.workspaces[0].prompt_panel.recent_scroll, 0);
    assert_eq!(
        state.workspaces[0].prompt_panel.recent_active,
        Some(0),
        "无当前编辑页时默认选中第一条"
    );
    assert!(state.prompt_resize_pending);
    assert!(toggle_prompt_recent(&mut state));
    assert!(!state.workspaces[0].prompt_panel.recent_open);
    assert_eq!(state.workspaces[0].prompt_panel.recent_active, None);
}

#[test]
fn open_recent_page_commits_current_and_switches_target() {
    let mut state = demo_state();
    state.workspaces[0].prompt_panel.recent_open = true;
    state.workspaces[0].prompt_panel.mode = PromptMode::Editor;
    state.workspaces[0].prompt_panel.editing = Some(PromptEditing::Item(10));
    state.prompt.set_text("alpha edited");

    let rows = prompt_recent_rows(&state);
    let index = rows
        .iter()
        .position(|row| row.page_id == PromptPageId::Saved(12))
        .expect("beta in recent rows");
    assert!(open_recent_page(&mut state, index));

    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(panel.mode, PromptMode::Editor);
    assert_eq!(panel.editing, Some(PromptEditing::Item(12)));
    assert!(!panel.recent_open, "选择后抽屉收起");
    assert_eq!(panel.recent_active, None);
    assert_eq!(panel.recent_scroll, 0);
    assert_eq!(panel.recent_visible, 0);
    assert_eq!(state.prompt.text(), "beta");
    // 旧页正文已提交到内存。
    let alpha = panel
        .tree
        .item(10)
        .and_then(|item| item.page(PromptPageId::Saved(10)))
        .expect("alpha page");
    assert_eq!(alpha.content, "alpha edited");
}

#[test]
fn selecting_current_page_closes_without_reload() {
    let mut state = demo_state();
    state.workspaces[0].prompt_panel.recent_open = true;
    state.workspaces[0].prompt_panel.mode = PromptMode::Editor;
    state.workspaces[0].prompt_panel.editing = Some(PromptEditing::Item(10));
    state.prompt.set_text("alpha edited");
    state.set_focus(FocusedContainer::Panes);
    let cursor = state.prompt.cursor();
    let revision = state.prompt.revision();
    let index = prompt_recent_rows(&state)
        .iter()
        .position(|row| row.page_id == PromptPageId::Saved(10))
        .expect("current page in recent rows");

    assert!(open_recent_page(&mut state, index));

    let panel = &state.workspaces[0].prompt_panel;
    assert!(!panel.recent_open, "当前页也收起抽屉");
    assert_eq!(panel.editing, Some(PromptEditing::Item(10)));
    assert_eq!(state.prompt.text(), "alpha edited", "当前页不重载正文");
    assert_eq!(state.prompt.cursor(), cursor, "当前页不移动光标");
    assert_eq!(state.prompt.revision(), revision, "当前页不重置修订");
    assert!(state.prompt_focused(), "关闭后键盘焦点回到 prompt");
}

#[test]
fn selecting_current_page_from_pages_returns_to_editor() {
    let mut state = demo_state();
    state.workspaces[0].prompt_panel.recent_open = true;
    state.workspaces[0].prompt_panel.mode = PromptMode::Pages;
    state.workspaces[0].prompt_panel.editing = Some(PromptEditing::Item(10));
    state.prompt.set_text("alpha");
    state.set_focus(FocusedContainer::Panes);
    let index = prompt_recent_rows(&state)
        .iter()
        .position(|row| row.page_id == PromptPageId::Saved(10))
        .expect("current page in recent rows");

    assert!(open_recent_page(&mut state, index));

    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(
        panel.mode,
        PromptMode::Editor,
        "从页列表选中当前页也要回到编辑视图"
    );
    assert!(!panel.recent_open, "选择后抽屉收起");
    assert_eq!(state.prompt.text(), "alpha", "当前页不重载正文");
    assert!(state.prompt_focused(), "选择后键盘焦点回到 prompt");
}

#[test]
fn opening_page_without_edit_keeps_recent_order() {
    let mut state = demo_state();
    state.workspaces[0].prompt_panel.recent_open = true;
    state.workspaces[0].prompt_panel.mode = PromptMode::Editor;
    state.workspaces[0].prompt_panel.editing = Some(PromptEditing::Item(10));
    state.prompt.set_text("alpha");

    let before: Vec<i64> = prompt_recent_rows(&state)
        .iter()
        .filter_map(|row| row.page_id.saved())
        .collect();
    assert_eq!(before, vec![12, 10], "Beta 更新时间更晚，应排在前");

    let beta = prompt_recent_rows(&state)
        .iter()
        .position(|row| row.page_id == PromptPageId::Saved(12))
        .expect("beta in recent rows");
    assert!(open_recent_page(&mut state, beta));
    assert_eq!(state.prompt.text(), "beta");

    let after: Vec<i64> = prompt_recent_rows(&state)
        .iter()
        .filter_map(|row| row.page_id.saved())
        .collect();
    assert_eq!(after, before, "仅打开不修改不得重排 recent");
    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(
        panel
            .tree
            .item(10)
            .and_then(|item| item.page(PromptPageId::Saved(10)))
            .map(|page| page.updated_at),
        Some(100),
        "未修改页的 updated_at 不变"
    );
    assert_eq!(
        panel
            .tree
            .item(12)
            .and_then(|item| item.page(PromptPageId::Saved(12)))
            .map(|page| page.updated_at),
        Some(200),
        "仅打开目标页也不刷新 updated_at"
    );
}

#[test]
fn sync_uses_cached_visible_entries() {
    let mut state = demo_state();
    state.workspaces[0]
        .prompt_panel
        .tree
        .item_mut(12)
        .expect("beta item")
        .pages
        .push(PromptPage::saved_at(13, "Page 2", "gamma", 300));
    {
        let panel = &mut state.workspaces[0].prompt_panel;
        panel.recent_open = true;
        panel.recent_visible = 2;
        panel.recent_active = Some(0);
        panel.recent_scroll = 0;
    }

    sync_prompt_recent_selection(&mut state, Some((10, PromptPageId::Saved(10))));

    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(panel.recent_active, Some(2), "选中项跟随目标页");
    assert_eq!(
        panel.recent_scroll, 1,
        "按实际可见 2 条而不是旧固定 3 条计算滚动窗口"
    );
}

#[test]
fn toggle_reduces_editor_viewport_after_resize() {
    let mut state = demo_state();
    let prompt = state.prompt.id();
    let panel = Rect::new(0, 0, 40, 20);
    crate::app::update::resize_panes(&mut state, &[(prompt, panel)]);
    let full_height = state.prompt.size().1;
    assert!(toggle_prompt_recent(&mut state));
    crate::app::update::resize_panes(&mut state, &[(prompt, panel)]);
    assert!(
        state.prompt.size().1 < full_height,
        "抽屉展开后编辑器视口必须收缩：{} -> {}",
        full_height,
        state.prompt.size().1
    );
    assert_eq!(
        state.workspaces[0].prompt_panel.recent_visible, 2,
        "resize 后缓存实际可见条目数"
    );
    assert!(!state.prompt_resize_pending, "resize 后清除待重算标记");

    assert!(close_prompt_recent(&mut state));
    crate::app::update::resize_panes(&mut state, &[(prompt, panel)]);
    assert_eq!(state.workspaces[0].prompt_panel.recent_visible, 0);
    assert_eq!(
        state.prompt.size().1,
        full_height,
        "收起抽屉后编辑器视口恢复完整高度"
    );
}

#[test]
fn scroll_is_clamped_to_entry_window() {
    let mut state = demo_state();
    state.workspaces[0].prompt_panel.recent_open = true;
    state.workspaces[0].prompt_panel.recent_active = Some(0);
    assert!(scroll_prompt_recent(&mut state, 1, 1, 3));
    assert_eq!(state.workspaces[0].prompt_panel.recent_scroll, 1);
    assert_eq!(
        state.workspaces[0].prompt_panel.recent_active,
        Some(1),
        "滚动后键盘选中项收进可见窗口"
    );
    assert!(scroll_prompt_recent(&mut state, 5, 1, 3));
    assert_eq!(
        state.workspaces[0].prompt_panel.recent_scroll, 2,
        "越界钳制到 total - visible"
    );
    assert_eq!(state.workspaces[0].prompt_panel.recent_active, Some(2));
    assert!(!scroll_prompt_recent(&mut state, 1, 1, 3));
    assert!(
        !set_prompt_recent_scroll(&mut state, 9, 1, 3),
        "已钳制在末尾"
    );
    assert_eq!(state.workspaces[0].prompt_panel.recent_scroll, 2);
}
