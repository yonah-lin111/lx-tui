//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::actions::OverlayKey;

#[test]
fn text_input_prefills_cursor_at_end() {
    let input = TextInput::new("workspace 2");
    assert_eq!(input.text(), "workspace 2");
    assert_eq!(input.cursor(), 11);
}

#[test]
fn text_input_edits_at_cursor() {
    let mut input = TextInput::new("ab");
    input.move_left();
    input.insert_char('X');
    assert_eq!(input.text(), "aXb");
    assert_eq!(input.cursor(), 2);
    input.backspace();
    assert_eq!(input.text(), "ab");
    input.delete();
    assert_eq!(input.text(), "a");
    input.move_home();
    input.delete();
    assert_eq!(input.text(), "");
    input.move_end();
    input.backspace();
    assert_eq!(input.text(), "");
    input.backspace();
    assert_eq!(input.cursor(), 0);
}

#[test]
fn text_input_handles_multibyte_characters() {
    let mut input = TextInput::new("工作区");
    input.backspace();
    assert_eq!(input.text(), "工作");
    input.move_home();
    input.insert_char('新');
    assert_eq!(input.text(), "新工作");
    assert_eq!(input.cursor(), 1);
    input.move_right();
    input.delete();
    assert_eq!(input.text(), "新工");
}

#[test]
fn text_input_movement_clamps_at_bounds() {
    let mut input = TextInput::new("");
    input.move_left();
    input.move_right();
    assert_eq!(input.cursor(), 0);
    input.insert_char('a');
    input.move_end();
    input.move_right();
    assert_eq!(input.cursor(), 1);
}

#[test]
fn text_input_selection_lifecycle_and_editing() {
    let mut input = TextInput::new("hello world");
    assert!(!input.has_selection());
    assert_eq!(input.selected_text(), None);

    // 拖拽选中 "world"
    input.begin_selection(6);
    input.drag_selection(11);
    assert!(input.has_selection());
    assert_eq!(input.selection(), Some((6, 11)));
    assert_eq!(input.selected_text().as_deref(), Some("world"));

    // 键入替换
    input.insert_char('!');
    assert_eq!(input.text(), "hello !");
    assert_eq!(input.cursor(), 7);
    assert!(!input.has_selection());

    // 选中并退格删除
    input.begin_selection(0);
    input.drag_selection(5);
    assert_eq!(input.selected_text().as_deref(), Some("hello"));
    input.backspace();
    assert_eq!(input.text(), " !");
    assert_eq!(input.cursor(), 0);
    assert!(!input.has_selection());

    // 选中并方向键折叠
    let mut input = TextInput::new("abcdef");
    input.set_selection(2, 4);
    assert_eq!(input.selected_text().as_deref(), Some("cd"));
    input.move_left();
    assert!(!input.has_selection());
    assert_eq!(input.cursor(), 2);

    input.set_selection(2, 4);
    input.move_right();
    assert!(!input.has_selection());
    assert_eq!(input.cursor(), 4);
}

#[test]
fn text_input_selection_clicks_word_and_all() {
    let mut input = TextInput::new("foo bar_baz 123");
    // 双击选词
    input.begin_selection_with_clicks(5, 2);
    assert_eq!(input.selected_text().as_deref(), Some("bar_baz"));

    // 三击全选
    input.begin_selection_with_clicks(1, 3);
    assert_eq!(input.selected_text().as_deref(), Some("foo bar_baz 123"));
}

#[test]
fn text_input_shortcuts_delete_to_line_start_and_end() {
    let mut input = TextInput::new("git commit -m fix");
    // 光标在 "commit" 前
    input.set_selection(0, 0);
    input.move_home();
    input.insert_str("test ");
    // test git commit -m fix
    // 移到中间
    input.set_selection(0, 0);
    input.clear_selection();
    for _ in 0..input.text().chars().count() {
        input.move_left();
    }
    for _ in 0..5 {
        input.move_right();
    }
    assert_eq!(input.cursor(), 5);

    // Ctrl+K 删至行尾
    input.delete_to_line_end();
    assert_eq!(input.text(), "test ");
    assert_eq!(input.cursor(), 5);

    // Ctrl+U 删至行首
    input.delete_to_line_start();
    assert_eq!(input.text(), "");
    assert_eq!(input.cursor(), 0);

    // move_home / move_end
    input.insert_str("hello");
    input.move_home();
    assert_eq!(input.cursor(), 0);
    input.move_end();
    assert_eq!(input.cursor(), 5);

    // clear
    input.clear();
    assert_eq!(input.text(), "");
    assert_eq!(input.cursor(), 0);
}

#[test]
fn text_input_undo_and_redo_lifecycle() {
    let mut input = TextInput::new("hello");
    assert!(!input.undo());

    input.insert_str(" world");
    assert_eq!(input.text(), "hello world");

    input.insert_char('!');
    assert_eq!(input.text(), "hello world!");

    // 撤销 '!'
    assert!(input.undo());
    assert_eq!(input.text(), "hello world");

    // 撤销 " world"
    assert!(input.undo());
    assert_eq!(input.text(), "hello");

    // 重做 " world"
    assert!(input.redo());
    assert_eq!(input.text(), "hello world");

    // 重做 '!'
    assert!(input.redo());
    assert_eq!(input.text(), "hello world!");

    // 清空与撤销
    input.clear();
    assert_eq!(input.text(), "");
    assert!(input.undo());
    assert_eq!(input.text(), "hello world!");
}

#[test]
fn text_input_selection_replace_is_single_undo_step() {
    let mut input = TextInput::new("hello world");
    input.set_selection(6, 11);
    input.insert_char('!');
    assert_eq!(input.text(), "hello !");

    // 撤销一步直接还原选区被替换前的 "hello world"
    assert!(input.undo());
    assert_eq!(input.text(), "hello world");
}

#[test]
fn overlay_kind_reports_variant() {
    let menu = Overlay::Menu(Menu {
        anchor: (0, 0),
        target: OverlayTarget::Workspace(0),
        commands: vec![MenuCommand::RenameWorkspace],
        selected: 0,
        confirming: false,
    });
    assert_eq!(menu.kind(), OverlayKind::Menu);
    let rename = Overlay::Rename(Rename {
        target: RenameTarget::Workspace(0),
        input: TextInput::new("a"),
    });
    assert_eq!(rename.kind(), OverlayKind::Rename);
    let confirm = Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Tab {
            workspace: 0,
            tab: 1,
        },
    });
    assert_eq!(confirm.kind(), OverlayKind::ConfirmClose);
    let new_ws = Overlay::NewWorkspace(NewWorkspace {
        input: TextInput::new(""),
    });
    assert_eq!(new_ws.kind(), OverlayKind::NewWorkspace);
    let worktree = Overlay::WorktreeOpen(worktree_dialog(vec![], 0));
    assert_eq!(worktree.kind(), OverlayKind::WorktreeOpen);
    let trust = Overlay::TrustWorkspace(TrustWorkspace {
        path: PathBuf::from("/tmp/ws"),
        trigger: crate::app::trust::TrustTrigger::CreateWorkspace {
            cwd: PathBuf::from("/tmp/ws"),
        },
    });
    assert_eq!(trust.kind(), OverlayKind::TrustWorkspace);
}

/// 构造测试对话框；条目按参数生成，源工作区为 0。
fn worktree_dialog(entries: Vec<WorktreeOpenEntry>, selected: usize) -> WorktreeOpen {
    WorktreeOpen {
        source: 0,
        repo_root: PathBuf::from("/repo"),
        entries,
        selected,
        query: TextInput::new(""),
        loading: false,
        failed: false,
    }
}

fn worktree_entry(path: &str, branch: Option<&str>, linked: bool) -> WorktreeOpenEntry {
    WorktreeOpenEntry {
        path: PathBuf::from(path),
        branch: branch.map(str::to_string),
        is_bare: false,
        is_linked: linked,
        already_open: None,
    }
}

#[test]
fn worktree_entry_display_name_prefers_branch_and_falls_back_to_directory() {
    let entry = worktree_entry("/repo/.worktrees/feat", Some("feature/x"), true);
    assert_eq!(entry.display_name(), "feature/x");
    let detached = worktree_entry("/repo/.worktrees/detached", None, true);
    assert_eq!(detached.display_name(), "detached");
}

#[test]
fn worktree_entry_status_reports_open_detached_root_and_branch() {
    let mut entry = worktree_entry("/repo", Some("main"), false);
    assert_eq!(entry.status(), WorktreeStatus::Branch);
    entry.already_open = Some(2);
    assert_eq!(entry.status(), WorktreeStatus::Open);
    let detached = worktree_entry("/repo/wt", None, true);
    assert_eq!(detached.status(), WorktreeStatus::Detached);
    let root = worktree_entry("/repo", None, false);
    assert_eq!(root.status(), WorktreeStatus::Root);
}

#[test]
fn worktree_dialog_filters_entries_by_query() {
    let mut dialog = worktree_dialog(
        vec![
            worktree_entry("/repo", Some("main"), false),
            worktree_entry("/repo/.worktrees/feature-x", Some("feature/x"), true),
            worktree_entry("/repo/.worktrees/notes", None, true),
        ],
        0,
    );
    assert_eq!(dialog.filtered_indices(), vec![0, 1, 2]);
    dialog.query = TextInput::new("feature");
    assert_eq!(dialog.filtered_indices(), vec![1]);
    dialog.query = TextInput::new("NOTES");
    assert_eq!(dialog.filtered_indices(), vec![2]);
    dialog.query = TextInput::new("zzz");
    assert!(dialog.filtered_indices().is_empty());
}

#[test]
fn worktree_dialog_normalizes_and_moves_selection_over_filtered_entries() {
    let mut dialog = worktree_dialog(
        vec![
            worktree_entry("/repo", Some("main"), false),
            worktree_entry("/repo/.worktrees/feature-x", Some("feature/x"), true),
            worktree_entry("/repo/.worktrees/notes", None, true),
        ],
        2,
    );
    dialog.query = TextInput::new("feature");
    dialog.normalize_selection();
    assert_eq!(dialog.selected_entry_index(), Some(1));

    dialog.select_next();
    assert_eq!(dialog.selected_entry_index(), Some(1));
    dialog.select_previous();
    assert_eq!(dialog.selected_entry_index(), Some(1));

    dialog.query = TextInput::new("");
    dialog.normalize_selection();
    assert_eq!(dialog.selected_entry_index(), Some(1));
    dialog.select_previous();
    assert_eq!(dialog.selected_entry_index(), Some(0));
    dialog.select_next();
    dialog.select_next();
    assert_eq!(dialog.selected_entry_index(), Some(2));
    dialog.select_next();
    assert_eq!(dialog.selected_entry_index(), Some(2));
}

#[test]
fn menu_command_requires_confirm_for_destructive_and_cwd_actions() {
    for cmd in [
        MenuCommand::CloseWorkspace,
        MenuCommand::CloseTab,
        MenuCommand::ClosePane,
        MenuCommand::DeleteFolder,
        MenuCommand::DeletePrompt,
        MenuCommand::SwitchToWorkspaceCwd,
        MenuCommand::SyncWorkspaceToTerminalCwd,
    ] {
        assert!(
            cmd.requires_confirm(),
            "{cmd:?} should require inline confirm"
        );
    }
    for cmd in [
        MenuCommand::NewTab,
        MenuCommand::NewTerminal,
        MenuCommand::NewWorkspace,
        MenuCommand::RenameWorkspace,
        MenuCommand::OpenWorktree,
        MenuCommand::OpenPrompt,
        MenuCommand::RenameTab,
        MenuCommand::SplitRight,
        MenuCommand::SplitDown,
        MenuCommand::SwitchToTerminal,
        MenuCommand::SwitchToLx,
        MenuCommand::NewPrompt,
        MenuCommand::NewFolder,
        MenuCommand::NewSubFolder,
        MenuCommand::RenameFolder,
        MenuCommand::RenamePrompt,
        MenuCommand::DeleteTemp,
        MenuCommand::EditPrompt,
        MenuCommand::CopyPromptContent,
        MenuCommand::CopyPath,
    ] {
        assert!(
            !cmd.requires_confirm(),
            "{cmd:?} should execute immediately without inline confirm"
        );
    }
}

#[test]
fn text_input_word_movement_and_deletion() {
    let mut input = TextInput::new("hello world foo");
    input.move_word_left();
    assert_eq!(input.cursor(), 12);
    input.move_word_left();
    assert_eq!(input.cursor(), 6);
    input.move_word_right();
    assert_eq!(input.cursor(), 11);
    input.move_word_right();
    assert_eq!(input.cursor(), 15);

    input.move_home();
    input.delete_word_forward();
    assert_eq!(input.text(), " world foo");
    input.move_end();
    input.delete_word_backward();
    assert_eq!(input.text(), " world ");
}

#[test]
fn text_input_word_deletion_prefers_selection() {
    let mut input = TextInput::new("hello world");
    input.set_selection(6, 11);
    input.delete_word_backward();
    assert_eq!(input.text(), "hello ");

    input.insert_str("world");
    input.set_selection(0, 6);
    input.delete_word_forward();
    assert_eq!(input.text(), "world");
}

#[test]
fn text_input_cut_selected_is_one_undo_step() {
    let mut input = TextInput::new("hello world");
    assert!(!input.can_undo());
    input.set_selection(6, 11);
    assert_eq!(input.cut_selected().as_deref(), Some("world"));
    assert_eq!(input.text(), "hello ");
    assert!(input.can_undo());
    assert!(input.undo());
    assert_eq!(input.text(), "hello world");
    input.reset_history();
    assert!(!input.can_undo());
    assert!(!input.undo());
}

#[test]
fn text_input_wrapped_up_down_moves_between_visual_rows() {
    let mut input = TextInput::new("abcdefghij");
    input.move_up_wrapped(5);
    assert_eq!(input.cursor(), 5);
    input.move_up_wrapped(5);
    assert_eq!(input.cursor(), 0);
    input.move_up_wrapped(5);
    assert_eq!(input.cursor(), 0);

    input.move_down_wrapped(5);
    assert_eq!(input.cursor(), 5);
    input.move_down_wrapped(5);
    assert_eq!(input.cursor(), 10);
    input.move_down_wrapped(5);
    assert_eq!(input.cursor(), 10);
}

#[test]
fn text_input_wrapped_up_down_respects_hard_breaks() {
    let mut input = TextInput::new("ab\ncdef");
    input.move_end();
    input.move_up_wrapped(5);
    assert_eq!(input.cursor(), 2);
    input.move_down_wrapped(5);
    assert_eq!(input.cursor(), 5);
}

#[test]
fn text_input_apply_edit_key_covers_word_and_history_commands() {
    let mut input = TextInput::new("one two");
    assert!(input.apply_edit_key(OverlayKey::WordLeft));
    assert_eq!(input.cursor(), 4);
    assert!(input.apply_edit_key(OverlayKey::DeleteWordBackward));
    assert_eq!(input.text(), "two");
    assert!(input.apply_edit_key(OverlayKey::Undo));
    assert_eq!(input.text(), "one two");
    assert!(input.apply_edit_key(OverlayKey::Redo));
    assert_eq!(input.text(), "two");
}
