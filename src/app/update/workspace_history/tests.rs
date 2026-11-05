use std::path::PathBuf;

use super::*;
use crate::app::overlay::{MenuCommand, Overlay, OverlayTarget, RenameTarget};
use crate::app::state::{FocusedContainer, WorkspaceGit};
use crate::app::trust::TrustTrigger;
use crate::app::update::{
    activate_menu, apply_overlay_key, begin_workspace_drag, begin_workspace_history_drag,
    drag_workspace_to, end_workspace_drag, end_workspace_history_drag, open_workspace_history_menu,
    open_workspace_menu,
};

fn plain_entry(name: &str, path: PathBuf, manual: bool) -> WorkspaceHistoryEntry {
    let exists = path.is_dir();
    WorkspaceHistoryEntry {
        name: name.into(),
        path,
        exists,
        name_is_manual: manual,
        position: 0,
        trusted: true,
        git: None,
    }
}

fn git_meta(
    repo_root: &std::path::Path,
    checkout_path: &std::path::Path,
    is_linked: bool,
) -> WorkspaceGit {
    WorkspaceGit {
        repo_root: repo_root.to_path_buf(),
        checkout_path: checkout_path.to_path_buf(),
        is_linked,
        branch: Some(if is_linked { "feature/x" } else { "dev" }.into()),
        main_branch: Some("dev".into()),
    }
}

fn git_entry(name: &str, path: PathBuf, position: i64, git: WorkspaceGit) -> WorkspaceHistoryEntry {
    let exists = path.is_dir();
    WorkspaceHistoryEntry {
        name: name.into(),
        path,
        exists,
        name_is_manual: false,
        position,
        trusted: true,
        git: Some(git),
    }
}

/// 构造「仓库父项 + linked 子项 + 独立工作区」三个活跃工作区与对应历史条目。
fn git_tree_fixture() -> (
    AppState,
    PathBuf,
    PathBuf,
    PathBuf,
    Vec<WorkspaceHistoryEntry>,
) {
    let repo = PathBuf::from("/tmp/lx-drag-repo");
    let child = repo.join(".worktrees").join("feat");
    let standalone = PathBuf::from("/tmp/lx-drag-standalone");

    let mut state = AppState::demo();
    state.workspaces[0].cwd = Some(repo.clone());
    state.workspaces[0].git = Some(git_meta(&repo, &repo, false));
    crate::app::update::create_workspace_with_cwd(&mut state, Some(child.clone()));
    state.workspaces[1].git = Some(git_meta(&repo, &child, true));
    crate::app::update::create_workspace_with_cwd(&mut state, Some(standalone.clone()));
    state.effects.clear();

    let entries = vec![
        git_entry("lx-tui", repo.clone(), 0, git_meta(&repo, &repo, false)),
        git_entry("feat", child.clone(), 1, git_meta(&repo, &child, true)),
        plain_entry("standalone", standalone.clone(), false),
    ];
    (state, repo, child, standalone, entries)
}

fn active_paths(state: &AppState) -> Vec<PathBuf> {
    state
        .workspaces
        .iter()
        .filter_map(|workspace| workspace.cwd.clone())
        .collect()
}

fn entry_paths(state: &AppState) -> Vec<PathBuf> {
    state
        .workspace_history
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect()
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
                position: 0,
                trusted: true,
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
                position: 1,
                trusted: true,
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

#[test]
fn apply_workspace_history_reorders_open_workspaces_to_match() {
    let (mut state, repo, child, standalone, entries) = git_tree_fixture();
    // 活跃列表构造顺序为 [父项, 子项, 独立项]；先把激活项切到子项，验证索引重映射。
    crate::app::update::switch_workspace(&mut state, 1);
    let active_path = state.active_workspace().cwd.clone();
    apply_workspace_history(&mut state, entries);

    // 历史顺序为 [父项, 子项, 独立项]，活跃列表已一致。
    assert_eq!(
        active_paths(&state),
        vec![repo.clone(), child.clone(), standalone]
    );
    assert_eq!(
        state.active_workspace().cwd.as_deref(),
        active_path.as_deref(),
        "激活项随重排指向同一工作区"
    );
}

#[test]
fn history_drag_moves_group_block_and_syncs_active_order() {
    let (mut state, repo, child, standalone, entries) = git_tree_fixture();
    apply_workspace_history(&mut state, entries);
    show_workspace_history(&mut state);

    // 拖拽独立项到仓库组之前。
    assert!(begin_workspace_history_drag(&mut state, 2));
    assert_eq!(state.workspace_history.drag, Some(2));
    assert!(!state.workspace_history.dragging);
    assert!(drag_workspace_history_to(&mut state, 0));
    assert!(state.workspace_history.dragging);
    assert_eq!(
        entry_paths(&state),
        vec![standalone.clone(), repo.clone(), child.clone()]
    );
    assert_eq!(
        active_paths(&state),
        vec![standalone.clone(), repo.clone(), child.clone()],
        "活跃列表同步为历史顺序"
    );
    assert_eq!(state.workspace_history.drag, Some(0));

    state.effects.clear();
    end_workspace_history_drag(&mut state);
    assert_eq!(state.workspace_history.drag, None);
    assert!(!state.workspace_history.dragging);
    let Some(Effect::SaveWorkspaceOrder { entries }) = state.effects.first() else {
        panic!("拖拽结束后必须持久化全局顺序: {:?}", state.effects);
    };
    let paths: Vec<&std::path::Path> = entries.iter().map(|(path, _)| path.as_path()).collect();
    assert_eq!(
        paths,
        vec![standalone.as_path(), repo.as_path(), child.as_path()]
    );
}

#[test]
fn active_drag_syncs_history_entries_and_persists() {
    let (mut state, repo, child, standalone, entries) = git_tree_fixture();
    apply_workspace_history(&mut state, entries);

    // 把独立项拖到最前。
    begin_workspace_drag(&mut state, 2);
    assert!(drag_workspace_to(&mut state, 0));
    assert_eq!(
        active_paths(&state),
        vec![standalone.clone(), repo.clone(), child.clone()]
    );

    state.effects.clear();
    end_workspace_drag(&mut state);
    assert_eq!(
        entry_paths(&state),
        vec![standalone.clone(), repo.clone(), child.clone()],
        "松开时历史条目同步为活跃顺序"
    );
    assert!(
        matches!(
            state.effects.as_slice(),
            [Effect::SaveWorkspaceOrder { entries }]
                if entries.iter().map(|(path, _)| path.as_path()).collect::<Vec<_>>()
                    == vec![standalone.as_path(), repo.as_path(), child.as_path()]
        ),
        "活跃列表拖拽结束后必须持久化全局顺序: {:?}",
        state.effects
    );
}

#[test]
fn history_drag_moves_whole_git_group_when_child_is_dragged() {
    let (mut state, repo, child, standalone, entries) = git_tree_fixture();
    apply_workspace_history(&mut state, entries);
    show_workspace_history(&mut state);

    // 拖拽子项（可见行 1）：整组（父项 + 子项）一起移到最后。
    assert!(begin_workspace_history_drag(&mut state, 1));
    assert!(drag_workspace_history_to(&mut state, 2));
    assert_eq!(
        entry_paths(&state),
        vec![standalone.clone(), repo.clone(), child.clone()]
    );
    assert_eq!(
        state.workspace_history.drag,
        Some(2),
        "拖动索引跟随组内被拖子项的新位置"
    );
}

#[test]
fn history_drag_is_ignored_while_search_filters_rows() {
    let (mut state, _repo, _child, _standalone, entries) = git_tree_fixture();
    apply_workspace_history(&mut state, entries);
    show_workspace_history(&mut state);
    state.workspace_history.search.insert_str("standalone");

    assert!(!begin_workspace_history_drag(&mut state, 0));
    assert_eq!(state.workspace_history.drag, None);
}

#[test]
fn untrusted_history_entry_requests_trust_instead_of_opening() {
    let mut state = AppState::demo();
    let dir = std::env::temp_dir().join(format!("lx-history-trust-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let mut entry = plain_entry("untrusted", dir.clone(), true);
    entry.trusted = false;
    apply_workspace_history(&mut state, vec![entry]);
    let before = state.workspaces.len();
    state.effects.clear();

    assert!(open_workspace_history_entry(&mut state, 0));
    assert_eq!(state.workspaces.len(), before, "信任检查前不得创建工作区");
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            Effect::CheckTrust {
                path,
                trigger: TrustTrigger::OpenWorkspaceHistory { .. },
                ..
            } if *path == dir
        )),
        "未信任条目必须先走 CheckTrust: {:?}",
        state.effects
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn opening_already_open_history_entry_returns_focus_to_panes() {
    let mut state = AppState::demo();
    let cwd = state.active_workspace().cwd.clone().expect("demo cwd");
    let entry = plain_entry("demo", cwd, true);
    apply_workspace_history(&mut state, vec![entry]);
    show_workspace_history(&mut state);
    assert_eq!(state.focus(), FocusedContainer::Sidebar);

    assert!(open_workspace_history_entry(&mut state, 0));
    assert_eq!(state.workspace_history.mode, WorkspaceMode::Active);
    assert_eq!(state.focus(), FocusedContainer::Panes);
}

#[test]
fn delete_worktree_count_reports_only_group_parent_children() {
    let (mut state, repo, child, standalone, entries) = git_tree_fixture();
    apply_workspace_history(&mut state, entries);
    let parent = state
        .workspace_history
        .entries
        .iter()
        .position(|entry| entry.path == repo)
        .expect("parent");
    let child_index = state
        .workspace_history
        .entries
        .iter()
        .position(|entry| entry.path == child)
        .expect("child");
    let standalone_index = state
        .workspace_history
        .entries
        .iter()
        .position(|entry| entry.path == standalone)
        .expect("standalone");

    assert_eq!(state.workspace_history.delete_worktree_count(parent), 1);
    assert_eq!(
        state.workspace_history.delete_worktree_count(child_index),
        0
    );
    assert_eq!(
        state
            .workspace_history
            .delete_worktree_count(standalone_index),
        0
    );
}

#[test]
fn clamp_limits_history_scroll_to_viewport() {
    use crate::app::state::WorkspaceHistoryPanel;

    let mut panel = WorkspaceHistoryPanel::new();
    panel.entries = (0..20)
        .map(|index| {
            plain_entry(
                &format!("ws-{index}"),
                PathBuf::from(format!("/tmp/ws-{index}")),
                false,
            )
        })
        .collect();
    panel.scroll = 18;
    assert!(panel.clamp(5));
    assert_eq!(
        panel.scroll, 15,
        "滚动上限必须是 total - viewport，而非 total - 1"
    );
}

#[test]
fn deleting_only_open_workspace_keeps_it_open_and_queues_delete() {
    let mut state = AppState::demo();
    let cwd = state.active_workspace().cwd.clone().expect("demo cwd");
    apply_workspace_history(&mut state, vec![plain_entry("demo", cwd.clone(), true)]);
    state.effects.clear();

    assert!(delete_workspace_history_entry(&mut state, 0));
    assert!(state.workspace_history.entries.is_empty());
    assert_eq!(state.workspaces.len(), 1, "最后一个工作区保持打开");
    assert!(
        state.effects.iter().any(
            |effect| matches!(effect, Effect::DeleteWorkspaceHistory { path } if *path == cwd)
        ),
        "必须登记数据库删除效应"
    );
}

#[test]
fn history_sync_keeps_unmatched_active_workspace_in_its_slot() {
    let base = std::env::temp_dir().join(format!("lx-history-slot-{}", std::process::id()));
    let a = base.join("a");
    let b = base.join("b");
    let x = base.join("x");
    for dir in [&a, &b, &x] {
        std::fs::create_dir_all(dir).expect("create dir");
    }
    let a = std::fs::canonicalize(&a).expect("canon a");
    let b = std::fs::canonicalize(&b).expect("canon b");
    let x = std::fs::canonicalize(&x).expect("canon x");

    let mut state = AppState::demo();
    state.workspaces[0].cwd = Some(a.clone());
    crate::app::update::create_workspace_with_cwd(&mut state, Some(b.clone()));
    crate::app::update::create_workspace_with_cwd(&mut state, Some(x.clone()));
    state.workspaces.swap(1, 2);
    state.effects.clear();

    apply_workspace_history(
        &mut state,
        vec![
            plain_entry("a", a.clone(), false),
            plain_entry("b", b.clone(), false),
        ],
    );

    assert_eq!(
        state
            .workspaces
            .iter()
            .map(|workspace| workspace.cwd.clone())
            .collect::<Vec<_>>(),
        vec![Some(a), Some(x), Some(b)],
        "未出现在历史中的工作区应保持原槽位，而不是被挤到末尾"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn history_drag_inside_same_block_does_not_persist_order() {
    let (mut state, _repo, _child, _standalone, entries) = git_tree_fixture();
    apply_workspace_history(&mut state, entries);
    show_workspace_history(&mut state);
    assert!(begin_workspace_history_drag(&mut state, 0));
    assert!(!drag_workspace_history_to(&mut state, 0));
    assert!(state.workspace_history.dragging, "指针移动后仍显示拖动反显");
    assert!(!state.workspace_history.drag_moved);
    state.effects.clear();
    end_workspace_history_drag(&mut state);
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::SaveWorkspaceOrder { .. })),
        "未改变顺序时松开不得写库"
    );
}
