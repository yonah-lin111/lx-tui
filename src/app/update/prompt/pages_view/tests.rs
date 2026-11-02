//! 页列表视图单元测试：切换矩阵、回车打开、右键菜单与增删改查。

use super::*;
use crate::app::prompt::{PromptItem, PromptPage};
use crate::app::update::{
    activate_menu, apply_editor, open_new_page, show_prompt_tree, toggle_prompt_panel_mode,
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

    // 9. Editor + Esc → Editor（Esc 不再进入 Tree，只有退出两个 list 的作用）
    apply_editor(&mut state, EditorCommand::Escape);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);

    // Pages 模式下点击 [≡]/[/] 按钮同样返回 Editor。
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

    // 右键第 3 页：不改变 selected（仍为 1），多页时包含 6 项命令。
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
            MenuCommand::ForkPage,
            MenuCommand::DeletePage,
        ]
    );

    // 只有一页时不包含 DeletePage（5 项）。
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
            MenuCommand::ForkPage,
        ]
    );
}

#[test]
fn page_menu_delete_requires_inline_confirm_and_preserves_undo_when_deleting_non_current_page() {
    let mut state = state_with_three_pages();
    state.prompt.insert_char('!');
    assert!(state.prompt.can_undo());

    show_prompt_pages(&mut state);
    state.workspaces[0].prompt_panel.pages_view.selected = 1;
    // 右键删除非选中页（第 3 页，id=12）：当前页是第 2 页（id=11），选中行保持第 2 页。
    open_page_menu(&mut state, PromptPageId::Saved(12), (10, 10));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 5; // DeletePage
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

    // 第二次触发：真正删除，留在 Pages 视图，选中行仍指向第 2 页 (index 1)，且撤销栈保留。
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
        menu.selected = 5;
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
    // 1. 正式条目 New page：留在 Pages 列表并推入 Effect::CreatePage，回调后不切到 Editor。
    let mut state = state_with_three_pages();
    show_prompt_pages(&mut state);
    open_pages_blank_menu(&mut state, (10, 10));
    state.effects.clear();
    activate_menu(&mut state);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Pages);
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::CreatePage { id: 10, .. }))
    );

    // 2. 临时提示词 New page：末尾追加新页并保持在 Pages 列表。
    let mut temp_state = AppState::demo();
    temp_state.prompt.set_text("temp p1");
    show_prompt_pages(&mut temp_state);
    open_pages_blank_menu(&mut temp_state, (10, 10));
    activate_menu(&mut temp_state);
    assert_eq!(
        temp_state.workspaces[0].prompt_panel.mode,
        PromptMode::Pages
    );
    assert_eq!(
        temp_state.workspaces[0].prompt_panel.temps()[0].pages.len(),
        2
    );
    assert_eq!(temp_state.prompt.text(), "temp p1");
    assert_eq!(
        temp_state.workspaces[0].prompt_panel.temps()[0].pages[1].content,
        ""
    );

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

#[test]
fn pages_menu_new_page_reuses_existing_empty_page_and_clears_search() {
    let mut state = state_with_three_pages();
    state.workspaces[0]
        .prompt_panel
        .tree
        .item_mut(10)
        .unwrap()
        .pages
        .push(PromptPage::saved(13, "Page 4", ""));
    show_prompt_pages(&mut state);
    apply_page_search(&mut state, &EditorCommand::InsertText("Second".into()));
    assert_eq!(
        state.workspaces[0].prompt_panel.pages_view.search.text(),
        "Second"
    );

    open_pages_blank_menu(&mut state, (10, 10));
    state.effects.clear();
    activate_menu(&mut state);

    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(panel.mode, PromptMode::Pages, "列表视图下不切回编辑视图");
    assert!(state.effects.is_empty(), "已有空白页时不得新建");
    assert!(state.toast.is_some(), "弹出 Already have a new page");
    assert_eq!(panel.pages_view.search.text(), "", "清空搜索过滤词");
    let (pages, active) = panel.editing_pages().unwrap();
    let rows = page_rows(pages, active, "");
    assert_eq!(
        rows[panel.pages_view.selected].page,
        PromptPageId::Saved(13),
        "选中既有空白页"
    );
}

#[test]
fn temp_pages_new_page_clears_search_and_selects_created_page() {
    let mut state = AppState::demo();
    state.prompt.set_text("temp p1");
    show_prompt_pages(&mut state);
    apply_page_search(&mut state, &EditorCommand::InsertText("zzz".into()));
    assert_eq!(
        page_rows(
            state.workspaces[0].prompt_panel.temps()[0].pages.as_slice(),
            state.workspaces[0].prompt_panel.temps()[0].active_page,
            state.workspaces[0].prompt_panel.pages_view.search.text(),
        )
        .len(),
        0
    );

    assert!(open_new_page(&mut state));
    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(panel.mode, PromptMode::Pages);
    assert_eq!(panel.pages_view.search.text(), "", "新建页后清空搜索");
    let (pages, active) = panel.editing_pages().unwrap();
    let rows = page_rows(pages, active, "");
    assert_eq!(
        rows[panel.pages_view.selected].page,
        pages.last().unwrap().id,
        "选中新建的末页"
    );
}

#[test]
fn delete_non_selected_page_above_selection_keeps_the_same_page_selected() {
    let mut state = state_with_three_pages();
    state.workspaces[0]
        .prompt_panel
        .tree
        .item_mut(10)
        .unwrap()
        .pages
        .push(PromptPage::saved(13, "Page 4", "{title: Fourth}"));
    show_prompt_pages(&mut state);
    state.workspaces[0].prompt_panel.pages_view.selected = 2; // 第 3 页 (id=12)

    open_page_menu(&mut state, PromptPageId::Saved(10), (10, 10));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 5; // DeletePage
    }
    activate_menu(&mut state);
    activate_menu(&mut state);

    let panel = &state.workspaces[0].prompt_panel;
    let (pages, active) = panel.editing_pages().unwrap();
    let rows = page_rows(pages, active, "");
    assert_eq!(
        rows[panel.pages_view.selected].page,
        PromptPageId::Saved(12),
        "删除选中行上方的页后选中箭头仍指向原页"
    );
}

#[test]
fn clearing_page_search_reselects_the_same_page() {
    let mut state = state_with_three_pages();
    show_prompt_pages(&mut state);
    // "i" 命中 First 与 Third，过滤列表为 [id=10, id=12]。
    apply_page_search(&mut state, &EditorCommand::InsertText("i".into()));
    assert_eq!(state.workspaces[0].prompt_panel.pages_view.selected, 0);
    state.workspaces[0].prompt_panel.pages_view.selected = 1; // 第 3 页 (id=12)

    assert!(clear_page_search(&mut state));
    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(panel.pages_view.search.text(), "");
    let (pages, active) = panel.editing_pages().unwrap();
    let rows = page_rows(pages, active, "");
    assert_eq!(
        rows[panel.pages_view.selected].page,
        PromptPageId::Saved(12),
        "清空搜索后选中行仍指向原页"
    );
}

#[test]
fn item_pages_mode_survives_workspace_round_trip() {
    let mut state = state_with_three_pages();
    show_prompt_pages(&mut state);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Pages);
    state
        .workspaces
        .push(crate::app::state::Workspace::single_terminal(
            "other".into(),
            Some(PathBuf::from("/other")),
        ));

    state.active_workspace = 1;
    crate::app::update::swap_display_prompt(&mut state, 0, 1);
    state.active_workspace = 0;
    crate::app::update::swap_display_prompt(&mut state, 1, 0);

    assert_eq!(
        state.workspaces[0].prompt_panel.mode,
        PromptMode::Pages,
        "正式条目切回后仍保持 Pages 视图"
    );
}

#[test]
fn editor_context_menu_lists_all_commands_and_hides_delete_when_single_page() {
    let mut state = state_with_three_pages();
    open_prompt_editor_menu(&mut state, (12, 8));
    let Some(Overlay::Menu(menu)) = &state.overlay else {
        panic!("expected editor menu");
    };
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::SelectAll,
            MenuCommand::CopyPageContent,
            MenuCommand::RenamePage,
            MenuCommand::NewPage,
            MenuCommand::ForkPage,
            MenuCommand::DeletePage,
        ]
    );

    let mut single = AppState::demo();
    open_prompt_editor_menu(&mut single, (12, 8));
    let Some(Overlay::Menu(menu)) = &single.overlay else {
        panic!("expected editor menu");
    };
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::SelectAll,
            MenuCommand::CopyPageContent,
            MenuCommand::RenamePage,
            MenuCommand::NewPage,
            MenuCommand::ForkPage,
        ]
    );
}

#[test]
fn ctrl_shift_right_advances_to_next_page_until_last_page_then_creates_new() {
    let mut state = AppState::demo();
    state.prompt.set_text("p1");
    assert!(crate::app::update::create_or_toast_new_page(&mut state));
    state.prompt.set_text("p2");
    assert!(crate::app::update::create_or_toast_new_page(&mut state));
    state.prompt.set_text("p3");
    assert!(crate::app::update::create_or_toast_new_page(&mut state));
    state.prompt.set_text("p4");
    assert!(crate::app::update::create_or_toast_new_page(&mut state));
    state.prompt.set_text("p5");

    // 切到第 3 页（共 5 页，索引 2）。
    let page3_id = state.workspaces[0].prompt_panel.temps()[0].pages[2].id;
    let page4_id = state.workspaces[0].prompt_panel.temps()[0].pages[3].id;
    let page5_id = state.workspaces[0].prompt_panel.temps()[0].pages[4].id;
    assert!(crate::app::update::select_prompt_page(&mut state, page3_id));
    assert_eq!(state.prompt.text(), "p3");

    // 在第 3 页按快捷键：切到第 4 页，页数保持 5。
    assert!(crate::app::update::advance_or_create_prompt_page(
        &mut state
    ));
    assert_eq!(
        state.workspaces[0].prompt_panel.temps()[0].active_page,
        page4_id
    );
    assert_eq!(state.prompt.text(), "p4");
    assert_eq!(state.workspaces[0].prompt_panel.temps()[0].pages.len(), 5);

    // 在第 4 页按快捷键：切到第 5 页，页数保持 5。
    assert!(crate::app::update::advance_or_create_prompt_page(
        &mut state
    ));
    assert_eq!(
        state.workspaces[0].prompt_panel.temps()[0].active_page,
        page5_id
    );
    assert_eq!(state.prompt.text(), "p5");
    assert_eq!(state.workspaces[0].prompt_panel.temps()[0].pages.len(), 5);

    // 在第 5 页（最后一页）按快捷键：新建第 6 页并切入。
    assert!(crate::app::update::advance_or_create_prompt_page(
        &mut state
    ));
    assert_eq!(state.workspaces[0].prompt_panel.temps()[0].pages.len(), 6);
    assert_eq!(state.prompt.text(), "");
}

#[test]
fn rename_page_scrolls_to_top_and_selects_title_value_or_handles_missing_title() {
    let mut state = AppState::demo();
    state.prompt.resize(40, 4);
    state.prompt.set_text(
        "&&& addTemplate --start\n{title: Login Refactor}\nl1\nl2\nl3\nl4\nl5\nl6\n&&& addTemplate --end",
    );
    state.prompt.scroll_to(3);
    assert!(state.prompt.scroll() > 0);

    open_prompt_editor_menu(&mut state, (10, 10));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 2; // RenamePage
    }
    activate_menu(&mut state);

    assert_eq!(state.prompt.scroll(), 0, "滚动条回到顶部");
    assert_eq!(
        crate::app::update::prompt_selection_text(&state).as_deref(),
        Some("Login Refactor"),
        "精准选中冒号后标题内容"
    );
    crate::app::update::apply_editor(&mut state, EditorCommand::InsertText("Auth Flow".into()));
    assert!(
        state
            .prompt
            .text()
            .starts_with("&&& addTemplate --start\n{title: Auth Flow}\n"),
        "直接输入即可替换选中标题: {}",
        state.prompt.text()
    );

    // 无模板块时弹 Warning Toast。
    state.prompt.set_text("plain text without template");
    state.toast = None;
    open_prompt_editor_menu(&mut state, (10, 10));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 2;
    }
    activate_menu(&mut state);
    assert_eq!(
        state.toast.as_ref().map(|t| t.message.as_str()),
        Some(crate::ui::text::TOAST_PAGE_NO_TITLE)
    );
}

#[test]
fn fork_page_clones_content_appends_fork_to_title_and_switches_to_new_page() {
    let mut state = AppState::demo();
    state
        .prompt
        .set_text("&&& addTemplate --start\n{title: My Task}\nbody line\n&&& addTemplate --end");

    open_prompt_editor_menu(&mut state, (10, 10));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 4; // ForkPage
    }
    activate_menu(&mut state);

    let temp = &state.workspaces[0].prompt_panel.temps()[0];
    assert_eq!(temp.pages.len(), 2, "追加新页");
    assert_eq!(temp.active_page, temp.pages[1].id, "立即切到新页");
    assert_eq!(
        state.prompt.text(),
        "&&& addTemplate --start\n{title: My Task (fork)}\nbody line\n&&& addTemplate --end"
    );
    assert_eq!(
        state.toast.as_ref().map(|t| t.message.as_str()),
        Some(crate::ui::text::TOAST_PAGE_FORKED)
    );
}

#[test]
fn editor_menu_select_all_focuses_prompt() {
    let mut state = state_with_three_pages();
    state.set_focus(crate::app::state::FocusedContainer::Panes);
    open_prompt_editor_menu(&mut state, (10, 10));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 0; // SelectAll
    }
    activate_menu(&mut state);

    assert!(state.prompt_focused(), "Select All 后焦点必须交给 prompt");
    assert!(state.selection.is_some(), "构造覆盖全文的选区");
}

#[test]
fn rename_page_selects_title_ending_with_variation_selector_without_crossing_brace() {
    let mut state = AppState::demo();
    state.prompt.resize(60, 6);
    state
        .prompt
        .set_text("&&& addTemplate --start\n{title: Heart ❤️}\nbody line\n&&& addTemplate --end");

    open_prompt_editor_menu(&mut state, (10, 10));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 2; // RenamePage
    }
    activate_menu(&mut state);

    let selected = crate::app::update::prompt_selection_text(&state).expect("标题选区");
    assert!(
        !selected.contains('}'),
        "选区不得越过右花括号：{selected:?}"
    );
    assert!(selected.contains('❤'), "选区覆盖标题值：{selected:?}");

    crate::app::update::apply_editor(&mut state, EditorCommand::InsertText("Beat".into()));
    let text = state.prompt.text();
    assert!(!text.contains('❤'), "打字替换标题值：{text:?}");
    assert!(
        text.contains("{title: Beat"),
        "替换文本写回标题行：{text:?}"
    );
    assert!(text.contains('}'), "右花括号保留：{text:?}");
}

#[test]
fn rename_page_keeps_title_cursor_visible_when_template_is_below_first_screen() {
    let mut state = AppState::demo();
    state.prompt.resize(40, 4);
    let mut text = String::new();
    for index in 0..12 {
        text.push_str(&format!("line {index}\n"));
    }
    text.push_str("&&& addTemplate --start\n{title: Deep Title}\nbody\n&&& addTemplate --end");
    state.prompt.set_text(&text);
    state.prompt.scroll_to(0);

    open_prompt_editor_menu(&mut state, (10, 10));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 2; // RenamePage
    }
    activate_menu(&mut state);

    assert!(state.prompt.scroll() > 0, "长正文下不得把标题滚出视口");
    assert!(
        state.prompt.cursor_cell().is_some(),
        "标题行必须留在视口内：scroll={}",
        state.prompt.scroll()
    );
    assert_eq!(
        crate::app::update::prompt_selection_text(&state).as_deref(),
        Some("Deep Title")
    );
}

#[test]
fn rename_page_inserts_title_line_with_start_line_indent_when_missing() {
    let mut state = AppState::demo();
    state.prompt.resize(60, 8);
    state
        .prompt
        .set_text("    &&& addTemplate --start\nbody line\n    &&& addTemplate --end");

    open_prompt_editor_menu(&mut state, (10, 10));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 2; // RenamePage
    }
    activate_menu(&mut state);

    assert!(
        state
            .prompt
            .text()
            .contains("    &&& addTemplate --start\n    {title: }"),
        "标题行继承起始行缩进：{:?}",
        state.prompt.text()
    );
    assert_eq!(state.prompt.scroll(), 0, "标题仍在首屏时保持顶部");
}

#[test]
fn fork_page_commits_dirty_saved_prompt_once() {
    let mut state = state_with_three_pages();
    state.prompt.insert_char('!');
    state.effects.clear();

    open_prompt_editor_menu(&mut state, (10, 10));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 4; // ForkPage
    }
    activate_menu(&mut state);

    let saves = state
        .effects
        .iter()
        .filter(|effect| matches!(effect, Effect::SavePromptContent { .. }))
        .count();
    assert_eq!(saves, 1, "Fork 只登记一次当前页保存：{:?}", state.effects);
}

#[test]
fn page_search_supports_paste_word_ops_and_cut() {
    let mut state = state_with_three_pages();
    assert!(show_prompt_pages(&mut state));
    assert!(apply_page_search(
        &mut state,
        &EditorCommand::InsertText("alpha beta".into())
    ));
    assert_eq!(
        state.workspaces[0].prompt_panel.pages_view.search.text(),
        "alpha beta"
    );

    assert!(apply_page_search(
        &mut state,
        &EditorCommand::DeleteWordBackward
    ));
    assert_eq!(
        state.workspaces[0].prompt_panel.pages_view.search.text(),
        "alpha "
    );
    assert!(apply_page_search(&mut state, &EditorCommand::WordLeft));
    assert_eq!(
        state.workspaces[0].prompt_panel.pages_view.search.cursor(),
        0
    );
    assert!(apply_page_search(
        &mut state,
        &EditorCommand::DeleteWordForward
    ));
    assert_eq!(
        state.workspaces[0].prompt_panel.pages_view.search.text(),
        " "
    );

    state.workspaces[0].prompt_panel.pages_view.search.clear();
    state.workspaces[0]
        .prompt_panel
        .pages_view
        .search
        .insert_str("alpha beta");
    state.workspaces[0]
        .prompt_panel
        .pages_view
        .search
        .set_selection(0, 5);
    assert_eq!(cut_page_search(&mut state).as_deref(), Some("alpha"));
    assert_eq!(
        state.workspaces[0].prompt_panel.pages_view.search.text(),
        " beta"
    );
    assert!(page_search_selection(&state).is_none());
}
