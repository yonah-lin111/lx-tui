//! 页列表视图单元测试：切换矩阵、回车打开、右键菜单与增删改查。

use super::*;
use crate::app::actions::OverlayKey;
use crate::app::prompt::{PromptItem, PromptPage};
use crate::app::update::{
    activate_menu, apply_editor, apply_overlay_key, show_prompt_tree, toggle_prompt_panel_mode,
};
use std::path::PathBuf;

fn state_with_three_pages() -> AppState {
    let mut state = AppState::demo();
    state.workspaces[0].cwd = Some(PathBuf::from("/repo"));
    let mut item = PromptItem::single(
        10,
        None,
        "My Item",
        "&&& addTemplate --start\n{title: First}\n&&& addTemplate --end",
    );
    item.pages.push(PromptPage::saved(
        11,
        "Page 2",
        "&&& bugTemplate --start\n{title: Second}\n&&& bugTemplate --end in_progress",
    ));
    item.pages.push(PromptPage::saved(
        12,
        "Page 3",
        "&&& refactorTemplate --start\n{title: Third}\n&&& refactorTemplate --end done",
    ));
    item.active_page = PromptPageId::Saved(11);
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    state.workspaces[0].prompt_panel.editing = Some(PromptEditing::Item(10));
    state.workspaces[0].prompt_panel.mode = PromptMode::Editor;
    state
        .prompt
        .set_text("&&& bugTemplate --start\n{title: Second}\n&&& bugTemplate --end in_progress");
    state
}

#[test]
fn mode_switching_matrix_covers_all_nine_combinations() {
    let mut state = state_with_three_pages();

    // 1. Editor + Ctrl+P → Pages
    apply_editor(&mut state, EditorCommand::TogglePageList);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Pages);
    assert_eq!(state.workspaces[0].prompt_panel.pages_view.selected, 1);

    // 2. Pages + Ctrl+P → Editor
    apply_editor(&mut state, EditorCommand::TogglePageList);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);

    // 3. Pages + Esc → Editor
    apply_editor(&mut state, EditorCommand::TogglePageList);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Pages);
    apply_editor(&mut state, EditorCommand::Escape);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);

    // 4. Pages + Ctrl+L → Tree
    apply_editor(&mut state, EditorCommand::TogglePageList);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Pages);
    apply_editor(&mut state, EditorCommand::TogglePromptList);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);

    // 5. Tree + Ctrl+P → 无反应（留在 Tree）
    apply_editor(&mut state, EditorCommand::TogglePageList);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);

    // 6. Tree + Ctrl+L → Editor
    apply_editor(&mut state, EditorCommand::TogglePromptList);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);

    // 7. Editor + Ctrl+L → Tree
    apply_editor(&mut state, EditorCommand::TogglePromptList);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);

    // 8. Tree + Esc → Editor
    apply_editor(&mut state, EditorCommand::Escape);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);

    // 9. Editor + Esc → Tree
    apply_editor(&mut state, EditorCommand::Escape);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);

    // Pages 模式下点击 [≡]/[/] 按钮同样返回 Editor。
    toggle_prompt_panel_mode(&mut state);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    show_prompt_pages(&mut state);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Pages);
    toggle_prompt_panel_mode(&mut state);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
}

#[test]
fn editor_to_pages_and_esc_preserves_cursor_undo_and_scroll() {
    let mut state = state_with_three_pages();
    state.prompt.resize(40, 5);
    for i in 0..10 {
        state.prompt.insert_str(&format!("\nline {i}"));
    }
    state.prompt.scroll_to(3);
    state.prompt.set_cursor_offset(10);
    let cursor = state.prompt.cursor();
    let scroll = state.prompt.scroll();
    assert!(state.prompt.can_undo());

    apply_editor(&mut state, EditorCommand::TogglePageList);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Pages);
    apply_editor(&mut state, EditorCommand::Escape);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert_eq!(state.prompt.cursor(), cursor);
    assert_eq!(state.prompt.scroll(), scroll);
    assert!(state.prompt.can_undo());
}

#[test]
fn enter_in_pages_opens_selected_page_or_returns_without_clearing_undo_on_same_page() {
    let mut state = state_with_three_pages();
    state.prompt.insert_char('!');
    assert!(state.prompt.can_undo());

    // Enter 打开当前页（第 2 页）：仅返回 Editor，撤销栈保留。
    apply_editor(&mut state, EditorCommand::TogglePageList);
    apply_editor(&mut state, EditorCommand::Newline);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert!(state.prompt.can_undo(), "同页返回不重载、保留撤销栈");

    // 再次进入 Pages，下移到第 3 页并按 Enter 打开：原页已 commit，Editor 载入第 3 页。
    apply_editor(&mut state, EditorCommand::TogglePageList);
    apply_editor(&mut state, EditorCommand::Down);
    assert_eq!(state.workspaces[0].prompt_panel.pages_view.selected, 2);
    apply_editor(&mut state, EditorCommand::Newline);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert!(state.prompt.text().contains("{title: Third}"));
    let page2 = &state.workspaces[0]
        .prompt_panel
        .tree
        .item(10)
        .unwrap()
        .pages[1]
        .content;
    assert!(page2.ends_with('!'), "离开前已 commit 原页修改");
}

#[test]
fn ctrl_l_closes_open_slash_panel_before_entering_tree() {
    let mut state = AppState::demo();
    apply_editor(&mut state, EditorCommand::InsertChar('/'));
    assert!(state.prompt.slash_panel().is_some());

    apply_editor(&mut state, EditorCommand::TogglePromptList);
    assert!(state.prompt.slash_panel().is_none());
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);
}

#[test]
fn page_context_menu_items_and_selection_preservation() {
    let mut state = state_with_three_pages();
    show_prompt_pages(&mut state);
    assert_eq!(state.workspaces[0].prompt_panel.pages_view.selected, 1);

    // 右键第 3 页：不改变 selected（仍为 1），多页时包含 5 项命令。
    open_page_menu(&mut state, PromptPageId::Saved(12), (10, 10));
    assert_eq!(state.workspaces[0].prompt_panel.pages_view.selected, 1);
    let Some(Overlay::Menu(menu)) = &state.overlay else {
        panic!("expected menu");
    };
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::OpenPage,
            MenuCommand::RenamePage,
            MenuCommand::CopyPageContent,
            MenuCommand::NewPage,
            MenuCommand::DeletePage,
        ]
    );

    // 只有一页时不包含 DeletePage（4 项）。
    let mut single = AppState::demo();
    show_prompt_pages(&mut single);
    let only_page = single.workspaces[0].prompt_panel.temps()[0].pages[0].id;
    open_page_menu(&mut single, only_page, (10, 10));
    let Some(Overlay::Menu(menu)) = &single.overlay else {
        panic!("expected menu");
    };
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::OpenPage,
            MenuCommand::RenamePage,
            MenuCommand::CopyPageContent,
            MenuCommand::NewPage,
        ]
    );
}

#[test]
fn page_menu_rename_validates_empty_and_pushes_effect() {
    let mut state = state_with_three_pages();
    // 第 2 页去掉 &&& 以验证回退页名。
    state.workspaces[0]
        .prompt_panel
        .tree
        .item_mut(10)
        .unwrap()
        .pages[1]
        .content = "plain body".to_string();
    state.prompt.set_text("plain body");
    show_prompt_pages(&mut state);
    open_page_menu(&mut state, PromptPageId::Saved(11), (10, 10));
    // 选中 RenamePage（索引 1）并执行。
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 1;
    }
    activate_menu(&mut state);
    let Some(Overlay::Rename(rename)) = &state.overlay else {
        panic!("expected Rename overlay");
    };
    assert_eq!(rename.input.text(), "Page 2");

    // 清空后提交：被拒绝，浮层保持打开。
    apply_overlay_key(&mut state, OverlayKey::Clear);
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert!(matches!(state.overlay, Some(Overlay::Rename(_))));

    // 输入新页名后提交：关闭浮层、推入 Effect::RenamePage，且页列表回退显示新页名。
    for ch in "Renamed Page".chars() {
        apply_overlay_key(&mut state, OverlayKey::Char(ch));
    }
    state.effects.clear();
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert!(state.overlay.is_none());
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::RenamePage { page_id: 11, name, .. } if name == "Renamed Page"
    )));
    let (pages, active) = state.workspaces[0].prompt_panel.editing_pages().unwrap();
    let rows = page_rows(pages, active, "");
    assert_eq!(rows[1].title, "Renamed Page");
}

#[test]
fn page_menu_delete_requires_inline_confirm_and_preserves_undo_when_deleting_non_current_page() {
    let mut state = state_with_three_pages();
    state.prompt.insert_char('!');
    assert!(state.prompt.can_undo());

    show_prompt_pages(&mut state);
    state.workspaces[0].prompt_panel.pages_view.selected = 2;
    // 右键删除非当前页（第 3 页，id=12）：当前页是第 2 页（id=11）。
    open_page_menu(&mut state, PromptPageId::Saved(12), (10, 10));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 4; // DeletePage
    }
    // 第一次触发：进入行内确认态，不删除。
    activate_menu(&mut state);
    assert!(
        matches!(&state.overlay, Some(Overlay::Menu(menu)) if menu.confirming),
        "第一次点击进入 confirming"
    );
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .tree
            .item(10)
            .unwrap()
            .pages
            .len(),
        3
    );

    // 第二次触发：真正删除，留在 Pages 视图，selected 钳到末项 (1)，且编辑缓冲撤销栈保留。
    activate_menu(&mut state);
    assert!(state.overlay.is_none());
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Pages);
    assert_eq!(state.workspaces[0].prompt_panel.pages_view.selected, 1);
    assert!(
        state.prompt.can_undo(),
        "删除非当前页不得冲掉编辑缓冲撤销栈"
    );

    // 删除当前页（第 2 页，id=11）：只剩第 1 页（id=10），active_page 变为 10，编辑缓冲载入第 1 页。
    open_page_menu(&mut state, PromptPageId::Saved(11), (10, 10));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 4;
    }
    activate_menu(&mut state);
    activate_menu(&mut state);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Pages);
    let item = state.workspaces[0].prompt_panel.tree.item(10).unwrap();
    assert_eq!(item.active_page, PromptPageId::Saved(10));
    assert!(state.prompt.text().contains("{title: First}"));
}

#[test]
fn page_menu_new_page_and_copy_content() {
    // 1. 正式条目 New page：返回 Editor 并推入 Effect::CreatePage，回调后不进入行内重命名。
    let mut state = state_with_three_pages();
    show_prompt_pages(&mut state);
    open_pages_blank_menu(&mut state, (10, 10));
    state.effects.clear();
    activate_menu(&mut state);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::CreatePage { id: 10, .. }))
    );

    // 2. 临时提示词 New page：末尾追加新页并回到 Editor。
    let mut temp_state = AppState::demo();
    temp_state.prompt.set_text("temp p1");
    show_prompt_pages(&mut temp_state);
    open_pages_blank_menu(&mut temp_state, (10, 10));
    activate_menu(&mut temp_state);
    assert_eq!(
        temp_state.workspaces[0].prompt_panel.mode,
        PromptMode::Editor
    );
    assert_eq!(
        temp_state.workspaces[0].prompt_panel.temps()[0].pages.len(),
        2
    );
    assert_eq!(temp_state.prompt.text(), "");

    // 3. Copy content：复制该页全文并弹出剪贴板 toast。
    show_prompt_pages(&mut state);
    open_page_menu(&mut state, PromptPageId::Saved(10), (10, 10));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 2; // CopyPageContent
    }
    state.effects.clear();
    activate_menu(&mut state);
    assert!(state.toast.is_some(), "弹出剪贴板 toast");
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::CopyToClipboard { text, .. } if text.contains("{title: First}")
    )));
}

#[test]
fn show_prompt_pages_returns_false_when_in_tree_mode() {
    let mut state = state_with_three_pages();
    show_prompt_tree(&mut state);
    assert!(!show_prompt_pages(&mut state));
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Tree);
}
