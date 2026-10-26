//! 单元测试；仅测试构建编译。

use super::*;

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
fn overlay_kind_reports_variant() {
    let menu = Overlay::Menu(Menu {
        anchor: (0, 0),
        target: OverlayTarget::Workspace(0),
        commands: vec![MenuCommand::RenameWorkspace],
        selected: 0,
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
    let switch_cwd = Overlay::ConfirmSwitchCwd(ConfirmSwitchCwd {
        workspace: 0,
        tab: 0,
        pane: PaneId::alloc(),
        path: PathBuf::from("/tmp/ws"),
    });
    assert_eq!(switch_cwd.kind(), OverlayKind::ConfirmSwitchCwd);
    let sync_cwd = Overlay::ConfirmSyncWorkspaceCwd(ConfirmSyncWorkspaceCwd {
        workspace: 0,
        pane: PaneId::alloc(),
        path: PathBuf::from("/tmp/terminal"),
    });
    assert_eq!(sync_cwd.kind(), OverlayKind::ConfirmSyncWorkspaceCwd);
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
fn text_input_select_all_and_replace_selection_is_one_undo_step() {
    let mut input = TextInput::new("hello world");
    input.select_all();
    assert_eq!(input.selected_text().as_deref(), Some("hello world"));

    input.insert_char('x');
    assert_eq!(input.text(), "x");
    assert_eq!(input.cursor(), 1);
    assert!(!input.has_selection());

    assert!(input.undo());
    assert_eq!(input.text(), "hello world");
    assert_eq!(input.selected_text().as_deref(), Some("hello world"));
}

#[test]
fn text_input_cut_returns_selection_and_keeps_one_undo_record() {
    let mut input = TextInput::new("abcdef");
    input.move_home();
    input.move_right();
    input.move_right();
    input.select_all();
    // select_all 覆盖整个文本；剪切后空。
    assert_eq!(input.cut_selection().as_deref(), Some("abcdef"));
    assert_eq!(input.text(), "");
    assert!(input.undo());
    assert_eq!(input.text(), "abcdef");
}

#[test]
fn text_input_word_movement_and_deletion_handle_cjk_and_punctuation() {
    let mut input = TextInput::new("hello 中文-world");
    input.move_end();
    input.move_word_left();
    assert_eq!(input.cursor(), 9, "跳到 world 词首");
    input.move_word_left();
    assert_eq!(input.cursor(), 6, "跳过连字符到中文词首");

    input.delete_word_backward();
    assert_eq!(input.text(), "中文-world");
    assert!(input.undo());
    assert_eq!(input.text(), "hello 中文-world");
}

#[test]
fn text_input_revision_tracks_edits_but_not_selection() {
    let mut input = TextInput::new("ab");
    let revision = input.revision();
    input.select_all();
    assert_eq!(input.revision(), revision, "选区不产生内容修订");
    input.insert_char('c');
    assert!(input.revision() > revision);
    let after = input.revision();
    input.move_left();
    assert_eq!(input.revision(), after);
}
