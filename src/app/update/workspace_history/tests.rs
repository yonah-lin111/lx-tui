use std::path::PathBuf;

use super::*;
use crate::app::overlay::{MenuCommand, Overlay, OverlayTarget, RenameTarget};
use crate::app::state::{FocusedContainer, WorkspaceGit};
use crate::app::update::{
    activate_menu, apply_overlay_key, open_workspace_history_menu, open_workspace_menu,
};

fn plain_entry(name: &str, path: PathBuf, manual: bool) -> WorkspaceHistoryEntry {
    let exists = path.is_dir();
    WorkspaceHistoryEntry {
        name: name.into(),
        path,
        exists,
        name_is_manual: manual,
        git: None,
    }
}

#[test]
fn toggle_workspace_mode_switches_and_focuses_sidebar() {
    let mut state = AppState::demo();
    assert!(!workspace_history_mode(&state));

    assert!(toggle_workspace_mode(&mut state));
    assert!(workspace_history_mode(&state));
    assert_eq!(state.focus(), FocusedContainer::Sidebar);
    assert!(
        state
            .effects
            .iter()
            .any(|eff| matches!(eff, Effect::LoadWorkspaceHistory))
    );

    assert!(toggle_workspace_mode(&mut state));
    assert!(!workspace_history_mode(&state));
}

#[test]
fn search_filtering_and_navigation_in_workspace_history() {
    let mut state = AppState::demo();
    let existing_dir = std::env::temp_dir();
    let missing_dir = PathBuf::from("/tmp/lx-nonexistent-history-dir-999999");
    apply_workspace_history(
        &mut state,
        vec![
            plain_entry("Alpha Project", existing_dir, true),
            plain_entry("Missing Project", missing_dir, true),
        ],
    );
    assert_eq!(state.workspace_history.entries.len(), 2);
    assert!(state.workspace_history.entries[0].exists);
    assert!(!state.workspace_history.entries[1].exists);

    show_workspace_history(&mut state);
    assert_eq!(state.workspace_history.filtered_indices(), vec![0, 1]);

    // Down / Up 移动选中项。
    assert!(move_workspace_history_selection(&mut state, 1, 5));
    assert_eq!(state.workspace_history.selected, 1);
    assert!(move_workspace_history_selection(&mut state, -1, 5));
    assert_eq!(state.workspace_history.selected, 0);

    // 搜索过滤（不区分大小写，匹配名称和路径）。
    for ch in "missing".chars() {
        apply_workspace_history_key(&mut state, OverlayKey::Char(ch), 5);
    }
    assert_eq!(state.workspace_history.filtered_indices(), vec![1]);
    assert_eq!(state.workspace_history.selected, 0);

    // 清空搜索恢复全部条目。
    assert!(clear_workspace_history_search(&mut state, 5));
    assert_eq!(state.workspace_history.filtered_indices(), vec![0, 1]);

    // Esc 退出历史模式。
    assert!(apply_workspace_history_key(&mut state, OverlayKey::Esc, 5));
    assert!(!workspace_history_mode(&state));
}

#[test]
fn git_tree_grouping_collapse_and_opening_linked_worktree_auto_opens_main_parent() {
    let mut state = AppState::demo();
    let repo_root = std::env::temp_dir();
    let wt_path = std::env::temp_dir().join(".");
    apply_workspace_history(
        &mut state,
        vec![
            WorkspaceHistoryEntry {
                name: "lx-tui".into(),
                path: repo_root.clone(),
                exists: true,
                name_is_manual: false,
                git: Some(WorkspaceGit {
                    repo_root: repo_root.clone(),
                    checkout_path: repo_root.clone(),
                    is_linked: false,
                    branch: Some("dev".into()),
                    main_branch: Some("dev".into()),
                }),
            },
            WorkspaceHistoryEntry {
                name: "workspace-history-list".into(),
                path: wt_path.clone(),
                exists: true,
                name_is_manual: false,
                git: Some(WorkspaceGit {
                    repo_root: repo_root.clone(),
                    checkout_path: wt_path.clone(),
                    is_linked: true,
                    branch: Some("feature/workspace-history-list".into()),
                    main_branch: Some("dev".into()),
                }),
            },
        ],
    );

    let rows = state.workspace_history.visible_rows();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].parent && !rows[0].collapsed);
    assert!(rows[1].child && rows[1].last_child);
    assert_eq!(
        state.workspace_history.entries[0].display_label(rows[0].child),
        "lx-tui"
    );
    assert_eq!(
        state.workspace_history.entries[1].display_label(rows[1].child),
        "feature/workspace-history-list"
    );

    // 点击父节点折叠/展开子工作区。
    assert!(toggle_workspace_history_group(&mut state, 0));
    let collapsed_rows = state.workspace_history.visible_rows();
    assert_eq!(collapsed_rows.len(), 1);
    assert!(collapsed_rows[0].parent && collapsed_rows[0].collapsed);

    assert!(toggle_workspace_history_group(&mut state, 0));
    assert_eq!(state.workspace_history.visible_rows().len(), 2);
}

#[test]
fn double_click_opens_existing_or_shows_toast_on_missing_directory() {
    let mut state = AppState::demo();
    let dir_a = std::env::temp_dir();
    let missing = PathBuf::from("/tmp/lx-nonexistent-history-dir-888888");
    apply_workspace_history(
        &mut state,
        vec![
            plain_entry("TempWS", dir_a.clone(), true),
            plain_entry("GoneWS", missing, true),
        ],
    );
    show_workspace_history(&mut state);

    // 双击第 1 行（路径不存在）：弹 Danger Toast 并留在历史列表模式。
    let now = Instant::now();
    assert!(!workspace_history_row_double_click(&mut state, 1, now));
    assert!(workspace_history_row_double_click(
        &mut state,
        1,
        now + Duration::from_millis(100)
    ));
    assert!(open_workspace_history_row(&mut state, 1));
    assert!(
        workspace_history_mode(&state),
        "路径不存在时保持在历史列表视图"
    );
    let toast = state.toast.as_ref().expect("danger toast shown");
    assert_eq!(toast.kind, ToastKind::Danger);
    assert_eq!(toast.message, text::TOAST_DIRECTORY_NOT_FOUND);

    // 双击第 0 行（路径存在且尚未在 state.workspaces 打开）：新建并激活该工作区，退回活跃列表。
    let now2 = now + Duration::from_secs(1);
    assert!(!workspace_history_row_double_click(&mut state, 0, now2));
    assert!(workspace_history_row_double_click(
        &mut state,
        0,
        now2 + Duration::from_millis(120)
    ));
    assert!(open_workspace_history_row(&mut state, 0));
    assert!(!workspace_history_mode(&state));
    assert_eq!(
        state.active_workspace().cwd.as_deref(),
        Some(dir_a.as_path())
    );
    assert_eq!(state.active_workspace().name, "TempWS");
    let count_after_first_open = state.workspaces.len();

    // 再次进入历史列表打开同一个已打开的路径：只切换过去，不重复创建。
    show_workspace_history(&mut state);
    assert!(open_workspace_history_row(&mut state, 0));
    assert!(!workspace_history_mode(&state));
    assert_eq!(state.workspaces.len(), count_after_first_open);
}

#[test]
fn context_menu_offers_delete_for_all_and_requires_3_clicks_plus_modal_and_interrupts_agents() {
    let mut state = AppState::demo();
    let existing_dir = std::env::temp_dir();
    let missing_dir = PathBuf::from("/tmp/lx-nonexistent-history-dir-777777");
    state.workspaces[0].cwd = Some(existing_dir.clone());
    // 模拟该活跃工作区中正在运行主 Agent 流、子 Agent 会话与本地 Shell 命令。
    let pane_id = state.workspaces[0].root_pane().expect("root pane");
    let active_key = crate::app::state::RequestKey {
        pane: pane_id,
        session: "sess-running-1".into(),
        generation: 1,
    };
    if let Some(pane) = state.pane_mut_anywhere(pane_id) {
        pane.lx.chat.session_id = Some("sess-running-1".into());
        pane.lx.chat.request = crate::app::state::RequestState::Streaming {
            key: active_key.clone(),
            assistant_id: "asst-1".into(),
        };
    }

    apply_workspace_history(
        &mut state,
        vec![
            plain_entry("ValidWS", existing_dir.clone(), true),
            plain_entry("BrokenWS", missing_dir, true),
        ],
    );
    show_workspace_history(&mut state);

    // 有效路径条目菜单：包含 Open、Rename 与 Delete（全部工作区均支持删除）。
    open_workspace_history_menu(&mut state, 0, (5, 5));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu expected");
    };
    assert_eq!(menu.target, OverlayTarget::WorkspaceHistory(0));
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::OpenWorkspaceHistory,
            MenuCommand::RenameWorkspace,
            MenuCommand::DeleteWorkspaceHistory
        ]
    );

    // 选中 Delete（下标 2）：需 3 次点击/回车 + 最后确认弹窗确认。
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 2;
    }
    state.effects.clear();

    // 第 1 次点击：变为 DeleteWorkspaceHistoryStep1 ("Confirm delete (1/2)")。
    activate_menu(&mut state);
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu stays open on step 1");
    };
    assert!(menu.confirming);
    assert_eq!(menu.commands[2], MenuCommand::DeleteWorkspaceHistoryStep1);
    assert_eq!(state.workspace_history.entries.len(), 2);

    // 第 2 次点击：变为 DeleteWorkspaceHistoryStep2 ("Confirm delete (2/2)")。
    activate_menu(&mut state);
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu stays open on step 2");
    };
    assert!(menu.confirming);
    assert_eq!(menu.commands[2], MenuCommand::DeleteWorkspaceHistoryStep2);
    assert_eq!(state.workspace_history.entries.len(), 2);

    // 第 3 次点击：关闭菜单并打开最终确认弹窗（ConfirmClose）。
    activate_menu(&mut state);
    assert!(matches!(
        state.overlay,
        Some(Overlay::ConfirmClose(ref c)) if c.target == OverlayTarget::WorkspaceHistory(0)
    ));
    assert_eq!(
        state.workspace_history.entries.len(),
        2,
        "最终弹窗确认前尚未删除"
    );

    // 最终弹窗确认（Enter）：中断运行中的 Agent 流、子 Agent 与本地命令，并登记级联删除效应。
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert!(state.overlay.is_none());
    assert_eq!(state.workspace_history.entries.len(), 1);
    assert!(
        state
            .effects
            .iter()
            .any(|eff| matches!(eff, Effect::CancelChatStream { key } if *key == active_key)),
        "必须中断正在运行的 Agent 流"
    );
    assert!(
        state.effects.iter().any(|eff| matches!(
            eff,
            Effect::CancelSubagents { session_id, agent_id: None } if session_id == "sess-running-1"
        )),
        "必须中断正在运行的子 Agent"
    );
    assert!(
        state
            .effects
            .iter()
            .any(|eff| matches!(eff, Effect::CancelUserShells { pane } if *pane == pane_id)),
        "必须中断正在运行的本地命令"
    );
    assert!(
        state.effects.iter().any(|eff| matches!(
            eff,
            Effect::DeleteWorkspaceHistory { path } if *path == existing_dir
        )),
        "必须登记数据库级联删除效应"
    );
}

#[test]
fn rename_syncs_between_active_workspace_and_history_list() {
    let mut state = AppState::demo();
    let path = PathBuf::from("/tmp/lx-rename-sync-test");
    state.workspaces[0].cwd = Some(path.clone());
    apply_workspace_history(
        &mut state,
        vec![plain_entry("OldName", path.clone(), false)],
    );

    // 从历史列表重命名：同步更新活跃工作区名称并登记 RenameWorkspace 效应。
    show_workspace_history(&mut state);
    open_workspace_history_menu(&mut state, 0, (4, 4));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 1;
    }
    activate_menu(&mut state);
    let Some(Overlay::Rename(rename)) = state.overlay.as_mut() else {
        panic!("rename overlay expected");
    };
    assert_eq!(rename.target, RenameTarget::WorkspaceHistory(0));
    rename.input.clear();
    rename.input.insert_str("NewHistoryName");
    state.effects.clear();
    apply_overlay_key(&mut state, OverlayKey::Enter);

    assert_eq!(state.workspace_history.entries[0].name, "NewHistoryName");
    assert!(state.workspace_history.entries[0].name_is_manual);
    assert_eq!(state.workspaces[0].name, "NewHistoryName");
    assert!(state.workspaces[0].name_is_manual);
    assert_eq!(
        state.effects,
        vec![Effect::RenameWorkspace {
            path: path.clone(),
            name: "NewHistoryName".into(),
        }]
    );

    // 从活跃列表重命名：同步更新历史列表条目名称并登记 RenameWorkspace 效应。
    exit_workspace_history(&mut state);
    open_workspace_menu(&mut state, 0, (4, 4));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = menu
            .commands
            .iter()
            .position(|cmd| *cmd == MenuCommand::RenameWorkspace)
            .unwrap_or(0);
    }
    activate_menu(&mut state);
    if let Some(Overlay::Rename(rename)) = state.overlay.as_mut() {
        rename.input.clear();
        rename.input.insert_str("ActiveRenamed");
    }
    state.effects.clear();
    apply_overlay_key(&mut state, OverlayKey::Enter);

    assert_eq!(state.workspaces[0].name, "ActiveRenamed");
    assert_eq!(state.workspace_history.entries[0].name, "ActiveRenamed");
    assert_eq!(
        state.effects,
        vec![Effect::RenameWorkspace {
            path,
            name: "ActiveRenamed".into(),
        }]
    );
}

#[test]
fn creating_workspace_in_history_mode_records_and_selects_without_opening() {
    let mut state = AppState::demo();
    let initial_ws_count = state.workspaces.len();
    let target_dir = std::env::temp_dir();
    apply_workspace_history(
        &mut state,
        vec![plain_entry(
            "OldEntry",
            PathBuf::from("/tmp/lx-old-entry"),
            true,
        )],
    );
    show_workspace_history(&mut state);
    state.workspace_history.search.insert_str("old");
    assert_eq!(state.workspace_history.selected, 0);

    // 在历史列表模式下打开新建工作区弹窗并提交路径（首个弹窗内二次确认：第 1 次回车进入待确认态，第 2 次回车直接确认，不再弹出 TrustWorkspace）。
    crate::app::update::open_new_workspace_dialog(&mut state);
    if let Some(Overlay::NewWorkspace(dialog)) = state.overlay.as_mut() {
        dialog.input.insert_str(&target_dir.to_string_lossy());
    }
    state.effects.clear();
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert!(
        matches!(state.overlay, Some(Overlay::NewWorkspace(_))),
        "第 1 次回车保持在首个弹窗内进行二次确认"
    );
    assert!(state.workspace_history.new_workspace_confirming);

    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert!(state.overlay.is_none(), "第 2 次回车完成确认并关闭弹窗");
    assert!(!state.workspace_history.new_workspace_confirming);

    // 保持停留在历史列表模式，不直接向活跃工作区列表添加/打开新工作区。
    assert!(
        workspace_history_mode(&state),
        "在 ws list 中新建后保持停留在历史列表"
    );
    assert_eq!(
        state.workspaces.len(),
        initial_ws_count,
        "不直接打开新建工作区"
    );
    assert!(
        state.workspace_history.search.text().is_empty(),
        "新建后自动清空搜索过滤以展示新条目"
    );
    let (_, selected_entry) = state
        .workspace_history
        .entry_at_visible_row(state.workspace_history.selected)
        .expect("selected entry");
    let canon_target = std::fs::canonicalize(&target_dir).unwrap_or(target_dir);
    assert_eq!(selected_entry.path, canon_target);
    assert!(
        state.effects.iter().any(
            |eff| matches!(eff, Effect::RecordWorkspace { path, .. } if *path == canon_target)
        )
    );
}
