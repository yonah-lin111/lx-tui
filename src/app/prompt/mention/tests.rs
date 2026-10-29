//! 文件提及与 refDir Tab 领域模型单元测试。

use std::path::PathBuf;

use super::*;
use crate::app::referenced::ReferencedFolder;

fn dummy_folders() -> Vec<ReferencedFolder> {
    vec![
        ReferencedFolder {
            id: 1,
            path: PathBuf::from("/Users/yonah/projects/herdr-master"),
        },
        ReferencedFolder {
            id: 2,
            path: PathBuf::from("/Users/yonah/projects/lx-agent"),
        },
    ]
}

#[test]
fn mention_tab_switching_cycles_between_files_and_ref_dir() {
    let mut state = MentionState::default();
    state.sync_referenced_folders(&dummy_folders());
    state.refresh("@", 1);
    assert!(state.view_open());
    assert_eq!(state.tab(), MentionTab::Files);

    // Right switches to refDir
    assert!(state.switch_tab(1));
    assert_eq!(state.tab(), MentionTab::RefDir);
    assert!(state.view_open());

    // Right wraps back to Files
    assert!(state.switch_tab(1));
    assert_eq!(state.tab(), MentionTab::Files);

    // Left wraps to refDir
    assert!(state.switch_tab(-1));
    assert_eq!(state.tab(), MentionTab::RefDir);

    // Direct set
    assert!(state.set_tab(MentionTab::Files));
    assert_eq!(state.tab(), MentionTab::Files);
    assert!(!state.set_tab(MentionTab::Files), "相同 tab 不重复切换");
}

#[test]
fn ref_dir_filters_referenced_folders_by_query() {
    let mut state = MentionState::default();
    state.sync_referenced_folders(&dummy_folders());

    // Empty query shows all
    state.refresh("@", 1);
    assert_eq!(state.filtered_ref_folders().len(), 2);

    // Filter by folder name
    state.refresh("@herdr", 6);
    let filtered = state.filtered_ref_folders();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, 1);

    // Filter by path segment (case-insensitive)
    state.refresh("@AGENT", 6);
    let filtered = state.filtered_ref_folders();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, 2);

    // No matches
    state.refresh("@unknown", 8);
    assert_eq!(state.filtered_ref_folders().len(), 0);
}

#[test]
fn ref_dir_confirm_inserts_refer_folder_and_resets_session() {
    let mut state = MentionState::default();
    state.sync_referenced_folders(&dummy_folders());

    let text = "see @agent for details";
    let cursor = 10; // after "@agent"
    state.refresh(text, cursor);
    assert!(state.switch_tab(1));
    assert_eq!(state.tab(), MentionTab::RefDir);

    let (range, insertion) = state.confirm(text.len()).expect("confirmed");
    assert_eq!(&text[range], "@agent");
    assert_eq!(
        insertion,
        "@[refer-folder](/Users/yonah/projects/lx-agent) "
    );

    // Session resets to Files
    assert_eq!(state.tab(), MentionTab::Files);
    assert!(!state.view_open());
}

#[test]
fn ref_dir_empty_state_prevents_confirm() {
    let mut state = MentionState::default();
    state.refresh("@nothing", 8);
    assert!(state.switch_tab(1));
    assert_eq!(state.filtered_ref_folders().len(), 0);

    // Confirm on empty list returns None
    assert_eq!(state.confirm(8), None);

    // enter_folder on refDir is always rejected
    assert_eq!(state.enter_folder(8), None);
}

#[test]
fn ref_dir_navigation_moves_and_clamps() {
    let mut state = MentionState::default();
    state.sync_referenced_folders(&dummy_folders());
    state.refresh("@", 1);
    assert!(state.switch_tab(1));

    assert_eq!(state.ref_active(), 0);
    assert!(state.move_active(1));
    assert_eq!(state.ref_active(), 1);

    // Wrap around
    assert!(state.move_active(1));
    assert_eq!(state.ref_active(), 0);

    // Wrap backwards
    assert!(state.move_active(-1));
    assert_eq!(state.ref_active(), 1);

    // Direct set
    assert!(state.set_active(0));
    assert_eq!(state.ref_active(), 0);
    assert!(!state.set_active(0), "相同索引不设值");
    assert!(!state.set_active(5), "越界不设值");
}

#[test]
fn files_empty_matches_keeps_view_open_allowing_tab_switch() {
    let mut state = MentionState::default();
    state.sync_referenced_folders(&dummy_folders());
    state.apply_entries(
        0,
        vec![MentionEntry {
            path: "src/main.rs".into(),
            is_directory: false,
        }],
        "@",
        1,
    );

    // Filter with "agent" -> 0 matches in Files
    state.refresh("@agent", 6);
    assert!(state.panel().is_none(), "Files 内无匹配条目");
    assert!(state.view_open(), "有缓存且会话激活时保持可视，允许切 Tab");

    // Can still switch to refDir and find the matching referenced folder
    assert!(state.switch_tab(1));
    assert_eq!(state.tab(), MentionTab::RefDir);
    assert_eq!(state.filtered_ref_folders().len(), 1);
    assert_eq!(state.filtered_ref_folders()[0].id, 2);
}

#[test]
fn inactive_mention_does_not_switch_tab() {
    let mut state = MentionState::default();
    state.refresh("plain text", 10);
    assert!(!state.view_open());
    assert!(!state.switch_tab(1), "非提及状态拒绝切 Tab");
    assert!(!state.set_tab(MentionTab::RefDir), "非提及状态拒绝设 Tab");
}

#[test]
fn ref_dir_viewport_clamps_on_query_narrowing() {
    let mut state = MentionState::default();
    state.sync_referenced_folders(&dummy_folders());
    state.refresh("@", 1);
    assert!(state.switch_tab(1));

    // Scroll viewport to 1
    assert!(state.scroll_viewport(1, 0));
    assert_eq!(state.ref_viewport(), Some(1));

    // Narrow query so count becomes 1 -> viewport resets safely
    state.refresh("@herdr", 6);
    assert_eq!(state.filtered_ref_folders().len(), 1);
    assert_eq!(state.ref_viewport(), None, "条目缩减时视口安全复位");
}
