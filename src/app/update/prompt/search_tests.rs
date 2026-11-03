//! 提示词树搜索框的粘贴、按词编辑与剪切测试。

use super::*;
use crate::app::actions::EditorCommand;
use crate::app::state::AppState;

#[test]
fn tree_search_supports_paste_word_ops_and_cut() {
    let mut state = AppState::demo();
    show_prompt_tree(&mut state);
    assert!(prompt_tree_mode(&state));
    assert!(apply_prompt_search(
        &mut state,
        &EditorCommand::InsertText("alpha beta".into())
    ));
    let panel = &state.workspaces[0].prompt_panel;
    assert_eq!(panel.search.text(), "alpha beta");

    assert!(apply_prompt_search(
        &mut state,
        &EditorCommand::DeleteWordBackward
    ));
    assert_eq!(state.workspaces[0].prompt_panel.search.text(), "alpha ");
    assert!(apply_prompt_search(&mut state, &EditorCommand::WordLeft));
    assert_eq!(state.workspaces[0].prompt_panel.search.cursor(), 0);
    assert!(apply_prompt_search(
        &mut state,
        &EditorCommand::DeleteWordForward
    ));
    assert_eq!(state.workspaces[0].prompt_panel.search.text(), " ");

    state.workspaces[0].prompt_panel.search.clear();
    state.workspaces[0]
        .prompt_panel
        .search
        .insert_str("alpha beta");
    state.workspaces[0].prompt_panel.search.set_selection(0, 5);
    assert_eq!(cut_prompt_search(&mut state).as_deref(), Some("alpha"));
    assert_eq!(state.workspaces[0].prompt_panel.search.text(), " beta");
    assert!(prompt_search_selection(&state).is_none());
}

#[test]
fn tree_search_select_all_covers_query() {
    let mut state = AppState::demo();
    show_prompt_tree(&mut state);
    assert!(apply_prompt_search(
        &mut state,
        &EditorCommand::InsertText("abc".into())
    ));
    assert!(apply_prompt_search(&mut state, &EditorCommand::SelectAll));
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .search
            .selected_text()
            .as_deref(),
        Some("abc")
    );
}
