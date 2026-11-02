//! 最近页面抽屉行为测试：开关、滚动钳制与单击跳转。

use super::*;
use crate::app::prompt::{PromptEditing, PromptPageId};
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
    assert!(state.prompt_resize_pending);
    assert!(toggle_prompt_recent(&mut state));
    assert!(!state.workspaces[0].prompt_panel.recent_open);
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
    assert!(panel.recent_open, "点击后抽屉保持展开");
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
    assert!(!state.prompt_resize_pending, "resize 后清除待重算标记");
}

#[test]
fn scroll_is_clamped_to_entry_window() {
    let mut state = demo_state();
    state.workspaces[0].prompt_panel.recent_open = true;
    assert!(scroll_prompt_recent(&mut state, 1, 1, 3));
    assert_eq!(state.workspaces[0].prompt_panel.recent_scroll, 1);
    assert!(scroll_prompt_recent(&mut state, 5, 1, 3));
    assert_eq!(
        state.workspaces[0].prompt_panel.recent_scroll, 2,
        "越界钳制到 total - visible"
    );
    assert!(!scroll_prompt_recent(&mut state, 1, 1, 3));
    assert!(
        !set_prompt_recent_scroll(&mut state, 9, 1, 3),
        "已钳制在末尾"
    );
    assert_eq!(state.workspaces[0].prompt_panel.recent_scroll, 2);
}
