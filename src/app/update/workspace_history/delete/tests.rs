use std::path::PathBuf;

use super::*;
use crate::app::actions::OverlayKey;
use crate::app::overlay::{ConfirmClose, MenuCommand, Overlay, OverlayTarget};
use crate::app::state::{WorkspaceGit, WorkspaceHistoryEntry};
use crate::app::update::{
    activate_menu, apply_overlay_key, apply_workspace_history, open_workspace_history_menu,
    open_workspace_list_menu, scroll_confirm_close, set_confirm_close_scroll,
    show_workspace_history,
};

fn plain_entry(name: &str, path: PathBuf) -> WorkspaceHistoryEntry {
    let exists = path.is_dir();
    WorkspaceHistoryEntry {
        name: name.into(),
        path,
        exists,
        name_is_manual: true,
        position: 0,
        trusted: true,
        git: None,
    }
}

fn git_entry(name: &str, path: PathBuf, git: WorkspaceGit) -> WorkspaceHistoryEntry {
    let exists = path.is_dir();
    WorkspaceHistoryEntry {
        name: name.into(),
        path,
        exists,
        name_is_manual: false,
        position: 0,
        trusted: true,
        git: Some(git),
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

fn type_text(state: &mut AppState, text: &str) {
    for ch in text.chars() {
        apply_overlay_key(state, OverlayKey::Char(ch));
    }
}

fn menu_commands(state: &AppState) -> Vec<MenuCommand> {
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    menu.commands.clone()
}

#[test]
fn context_menu_delete_requires_two_clicks_and_typed_phrase() {
    let mut state = AppState::demo();
    let existing_dir = std::env::temp_dir();
    let missing_dir = std::env::temp_dir().join(format!(
        "lx-nonexistent-history-dir-{}-confirm",
        std::process::id()
    ));
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
            plain_entry("ValidWS", existing_dir.clone()),
            plain_entry("BrokenWS", missing_dir),
        ],
    );
    show_workspace_history(&mut state);

    // 存在失效条目时条目菜单底部追加 Delete missing。
    open_workspace_history_menu(&mut state, 0, (5, 5));
    assert_eq!(
        menu_commands(&state),
        vec![
            MenuCommand::OpenWorkspaceHistory,
            MenuCommand::RenameWorkspace,
            MenuCommand::DeleteWorkspaceHistory,
            MenuCommand::DeleteMissingWorkspaces,
        ]
    );
    let Some(Overlay::Menu(menu)) = state.overlay.as_mut() else {
        panic!("menu expected");
    };
    assert_eq!(menu.target, OverlayTarget::WorkspaceHistory(0));
    menu.selected = 2;
    state.effects.clear();

    // 第 1 次点击仅进入行内确认态，菜单与条目保持不变。
    activate_menu(&mut state);
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu stays open on first click");
    };
    assert!(menu.confirming);
    assert_eq!(menu.commands[2], MenuCommand::DeleteWorkspaceHistory);
    assert_eq!(state.workspace_history.entries.len(), 2);

    // 第 2 次点击关闭菜单并打开输入口令的确认弹窗。
    activate_menu(&mut state);
    assert!(matches!(
        state.overlay,
        Some(Overlay::ConfirmClose(ref c)) if c.target == OverlayTarget::WorkspaceHistory(0)
    ));
    assert_eq!(
        state.workspace_history.entries.len(),
        2,
        "口令确认前尚未删除"
    );

    // 未输入口令按 Enter：弹窗保持打开、条目不删除、不产生删除效应。
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert!(matches!(state.overlay, Some(Overlay::ConfirmClose(_))));
    assert_eq!(state.workspace_history.entries.len(), 2);
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::DeleteWorkspaceHistory { .. })),
        "口令未匹配时不得登记删除效应"
    );

    // 输入错误口令同样被拦截。
    type_text(&mut state, "delete");
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert!(matches!(state.overlay, Some(Overlay::ConfirmClose(_))));
    assert_eq!(state.workspace_history.entries.len(), 2);

    // 口令忽略首尾空白与 ASCII 大小写："  CONFIRM DELETE " 通过。
    if let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_mut() {
        confirm.input.clear();
    }
    type_text(&mut state, "  CONFIRM DELETE ");
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
fn delete_missing_menu_visibility_follows_missing_entries() {
    let mut state = AppState::demo();
    let base = std::env::temp_dir().join(format!("lx-missing-menu-{}", std::process::id()));
    let existing = base.join("existing");
    std::fs::create_dir_all(&existing).expect("create existing dir");
    let missing = base.join("missing");

    apply_workspace_history(
        &mut state,
        vec![
            plain_entry("Existing", existing.clone()),
            plain_entry("Missing", missing),
        ],
    );

    // 活跃工作区视图：空白菜单始终只有 New workspace。
    open_workspace_list_menu(&mut state, (0, 0));
    assert_eq!(menu_commands(&state), vec![MenuCommand::NewWorkspace]);

    // 历史视图且存在失效条目：空白菜单与条目菜单都追加 Delete missing。
    show_workspace_history(&mut state);
    open_workspace_list_menu(&mut state, (0, 0));
    assert_eq!(
        menu_commands(&state),
        vec![
            MenuCommand::NewWorkspace,
            MenuCommand::DeleteMissingWorkspaces
        ]
    );
    open_workspace_history_menu(&mut state, 0, (0, 0));
    assert_eq!(
        menu_commands(&state),
        vec![
            MenuCommand::OpenWorkspaceHistory,
            MenuCommand::RenameWorkspace,
            MenuCommand::DeleteWorkspaceHistory,
            MenuCommand::DeleteMissingWorkspaces,
        ]
    );

    // 全部条目存在：两种菜单都不显示 Delete missing。
    apply_workspace_history(
        &mut state,
        vec![
            plain_entry("Existing", existing.clone()),
            plain_entry("Existing again", std::env::temp_dir()),
        ],
    );
    open_workspace_list_menu(&mut state, (0, 0));
    assert_eq!(menu_commands(&state), vec![MenuCommand::NewWorkspace]);
    open_workspace_history_menu(&mut state, 0, (0, 0));
    assert_eq!(
        menu_commands(&state),
        vec![
            MenuCommand::OpenWorkspaceHistory,
            MenuCommand::RenameWorkspace,
            MenuCommand::DeleteWorkspaceHistory,
        ]
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn delete_missing_menu_requires_two_clicks_and_opens_confirm_overlay() {
    let mut state = AppState::demo();
    let missing = std::env::temp_dir().join(format!(
        "lx-nonexistent-delete-missing-{}",
        std::process::id()
    ));
    apply_workspace_history(&mut state, vec![plain_entry("Missing", missing)]);
    show_workspace_history(&mut state);

    // 空白右键菜单：第 1 次点击仅进入确认态，第 2 次打开 MissingWorkspaces 口令弹窗。
    open_workspace_list_menu(&mut state, (0, 0));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 1;
    }
    activate_menu(&mut state);
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu stays open on first click");
    };
    assert!(menu.confirming);
    activate_menu(&mut state);
    assert!(matches!(
        state.overlay,
        Some(Overlay::ConfirmClose(ref c)) if c.target == OverlayTarget::MissingWorkspaces
    ));

    // 条目右键菜单同样支持。
    state.overlay = None;
    open_workspace_history_menu(&mut state, 0, (0, 0));
    if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
        menu.selected = 3;
    }
    activate_menu(&mut state);
    activate_menu(&mut state);
    assert!(matches!(
        state.overlay,
        Some(Overlay::ConfirmClose(ref c)) if c.target == OverlayTarget::MissingWorkspaces
    ));
}

#[test]
fn delete_missing_removes_only_missing_entries_across_full_history() {
    let mut state = AppState::demo();
    let base = std::env::temp_dir().join(format!("lx-missing-cleanup-{}", std::process::id()));
    let repo = base.join("repo");
    let child = repo.join("wt-existing");
    let missing_wt = repo.join("wt-missing");
    let orphan = base.join("orphan-missing");
    let missing_repo = base.join("repo-missing");
    let existing_child_of_missing_repo = base.join("existing-child");
    std::fs::create_dir_all(&child).expect("create repo worktree");
    std::fs::create_dir_all(&existing_child_of_missing_repo)
        .expect("create existing child worktree");

    apply_workspace_history(
        &mut state,
        vec![
            git_entry("repo", repo.clone(), git_meta(&repo, &repo, false)),
            git_entry("wt-existing", child.clone(), git_meta(&repo, &child, true)),
            git_entry(
                "wt-missing",
                missing_wt.clone(),
                git_meta(&repo, &missing_wt, true),
            ),
            plain_entry("orphan-missing", orphan.clone()),
            git_entry(
                "repo-missing",
                missing_repo.clone(),
                git_meta(&missing_repo, &missing_repo, false),
            ),
            git_entry(
                "existing-child",
                existing_child_of_missing_repo.clone(),
                git_meta(&missing_repo, &existing_child_of_missing_repo, true),
            ),
        ],
    );
    // 顶部搜索过滤只命中部分条目，批量清理仍须作用于全量历史列表。
    show_workspace_history(&mut state);
    state.workspace_history.search.insert_str("repo");
    state.effects.clear();

    assert_eq!(delete_missing_workspace_history_entries(&mut state), 3);

    let remaining: Vec<PathBuf> = state
        .workspace_history
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    assert_eq!(
        remaining,
        vec![
            repo.clone(),
            child.clone(),
            existing_child_of_missing_repo.clone()
        ],
        "只保留磁盘上仍存在的条目（含存活主仓库与 linked worktree）"
    );
    assert!(!remaining.contains(&missing_wt));
    assert!(!remaining.contains(&orphan));
    assert!(!remaining.contains(&missing_repo));

    let deleted: Vec<PathBuf> = state
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::DeleteWorkspaceHistory { path } => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        deleted,
        vec![missing_wt, orphan, missing_repo],
        "只为失效路径登记数据库删除效应"
    );
    assert!(
        state
            .workspace_history
            .hidden_worktrees
            .contains(&repo.join("wt-missing")),
        "失效的 linked worktree 必须登记防复活"
    );
    assert!(
        !state
            .workspace_history
            .hidden_worktrees
            .contains(&existing_child_of_missing_repo),
        "存活的 linked worktree 不得被登记为隐藏"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn delete_missing_interrupts_and_closes_open_missing_workspace() {
    let mut state = AppState::demo();
    let missing = std::env::temp_dir().join(format!(
        "lx-nonexistent-open-workspace-{}",
        std::process::id()
    ));
    state.workspaces[0].cwd = Some(missing.clone());
    let pane_id = state.workspaces[0].root_pane().expect("root pane");
    let active_key = crate::app::state::RequestKey {
        pane: pane_id,
        session: "sess-missing-1".into(),
        generation: 1,
    };
    if let Some(pane) = state.pane_mut_anywhere(pane_id) {
        pane.lx.chat.session_id = Some("sess-missing-1".into());
        pane.lx.chat.request = crate::app::state::RequestState::Streaming {
            key: active_key.clone(),
            assistant_id: "asst-missing".into(),
        };
    }
    apply_workspace_history(&mut state, vec![plain_entry("Missing", missing.clone())]);
    state.effects.clear();

    assert_eq!(delete_missing_workspace_history_entries(&mut state), 1);
    assert!(state.workspace_history.entries.is_empty());
    assert_eq!(state.workspaces.len(), 1, "最后一个工作区保持打开");
    assert!(
        state
            .effects
            .iter()
            .any(|eff| matches!(eff, Effect::CancelChatStream { key } if *key == active_key))
    );
    assert!(state.effects.iter().any(|eff| matches!(
        eff,
        Effect::CancelSubagents { session_id, agent_id: None } if session_id == "sess-missing-1"
    )));
    assert!(
        state
            .effects
            .iter()
            .any(|eff| matches!(eff, Effect::CancelUserShells { pane } if *pane == pane_id))
    );
    assert!(state.effects.iter().any(
        |effect| matches!(effect, Effect::DeleteWorkspaceHistory { path } if *path == missing)
    ));
}

#[test]
fn delete_main_repo_entry_cascades_linked_worktrees() {
    let mut state = AppState::demo();
    let base = std::env::temp_dir().join(format!("lx-delete-repo-{}", std::process::id()));
    let repo = base.join("repo");
    let child = repo.join("wt");
    std::fs::create_dir_all(&child).expect("create worktree");

    apply_workspace_history(
        &mut state,
        vec![
            git_entry("repo", repo.clone(), git_meta(&repo, &repo, false)),
            git_entry("wt", child.clone(), git_meta(&repo, &child, true)),
        ],
    );
    state.effects.clear();

    assert!(delete_workspace_history_entry(&mut state, 0));
    assert!(state.workspace_history.entries.is_empty());
    let deleted: Vec<PathBuf> = state
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::DeleteWorkspaceHistory { path } => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        deleted,
        vec![repo, child],
        "主仓库删除须级联 linked worktree"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn deleting_only_open_workspace_keeps_it_open_and_queues_delete() {
    let mut state = AppState::demo();
    let cwd = state.active_workspace().cwd.clone().expect("demo cwd");
    apply_workspace_history(&mut state, vec![plain_entry("demo", cwd.clone())]);
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
fn delete_missing_without_missing_entries_is_noop() {
    let mut state = AppState::demo();
    apply_workspace_history(
        &mut state,
        vec![plain_entry("Existing", std::env::temp_dir())],
    );
    state.effects.clear();

    assert_eq!(delete_missing_workspace_history_entries(&mut state), 0);
    assert_eq!(state.workspace_history.entries.len(), 1);
    assert!(state.effects.is_empty());
}

fn confirm_scroll(state: &AppState) -> Option<usize> {
    state.overlay.as_ref().and_then(|overlay| match overlay {
        Overlay::ConfirmClose(confirm) => Some(confirm.scroll),
        _ => None,
    })
}

fn missing_entries(count: usize, label: &str) -> Vec<WorkspaceHistoryEntry> {
    let base = std::env::temp_dir().join(format!("lx-{label}-{}", std::process::id()));
    (0..count)
        .map(|index| plain_entry(&format!("Gone{index}"), base.join(format!("gone-{index}"))))
        .collect()
}

#[test]
fn target_delete_entries_single_workspace_returns_only_itself() {
    let mut state = AppState::demo();
    let path = std::env::temp_dir().join(format!("lx-target-delete-single-{}", std::process::id()));
    apply_workspace_history(&mut state, vec![plain_entry("Solo", path.clone())]);

    assert_eq!(
        state
            .workspace_history
            .target_delete_entries(&OverlayTarget::WorkspaceHistory(0)),
        vec![("Solo".to_string(), path)]
    );
    // 越界索引与不可删除目标返回空列表。
    assert!(
        state
            .workspace_history
            .target_delete_entries(&OverlayTarget::WorkspaceHistory(9))
            .is_empty()
    );
    assert!(
        state
            .workspace_history
            .target_delete_entries(&OverlayTarget::WorkspaceList)
            .is_empty()
    );
}

#[test]
fn target_delete_entries_git_parent_includes_linked_worktrees() {
    let mut state = AppState::demo();
    let base = std::env::temp_dir().join(format!("lx-target-delete-repo-{}", std::process::id()));
    let repo = base.join("repo");
    let child = repo.join("wt");
    std::fs::create_dir_all(&child).expect("create worktree");
    apply_workspace_history(
        &mut state,
        vec![
            git_entry("repo", repo.clone(), git_meta(&repo, &repo, false)),
            git_entry("wt", child.clone(), git_meta(&repo, &child, true)),
        ],
    );

    assert_eq!(
        state
            .workspace_history
            .target_delete_entries(&OverlayTarget::WorkspaceHistory(0)),
        vec![
            ("repo".to_string(), repo.clone()),
            ("feature/x".to_string(), child.clone()),
        ]
    );
    // linked worktree 自身只返回自身，不反向级联主仓库。
    assert_eq!(
        state
            .workspace_history
            .target_delete_entries(&OverlayTarget::WorkspaceHistory(1)),
        vec![("feature/x".to_string(), child)]
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn target_delete_entries_missing_workspaces_returns_all_missing() {
    let mut state = AppState::demo();
    let base =
        std::env::temp_dir().join(format!("lx-target-delete-missing-{}", std::process::id()));
    let existing = base.join("existing");
    std::fs::create_dir_all(&existing).expect("create existing");
    let missing_a = base.join("missing-a");
    let missing_b = base.join("missing-b");
    apply_workspace_history(
        &mut state,
        vec![
            plain_entry("Existing", existing),
            plain_entry("GoneA", missing_a.clone()),
            plain_entry("GoneB", missing_b.clone()),
        ],
    );

    assert_eq!(
        state
            .workspace_history
            .target_delete_entries(&OverlayTarget::MissingWorkspaces),
        vec![
            ("GoneA".to_string(), missing_a),
            ("GoneB".to_string(), missing_b),
        ]
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn confirm_close_scroll_clamps_within_list_bounds() {
    let mut state = AppState::demo();
    apply_workspace_history(&mut state, missing_entries(8, "confirm-scroll"));
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(
        OverlayTarget::MissingWorkspaces,
    )));
    assert_eq!(confirm_scroll(&state), Some(0));

    // 8 条 - 5 行 = 最大偏移 3。
    for _ in 0..10 {
        scroll_confirm_close(&mut state, 1);
    }
    assert_eq!(confirm_scroll(&state), Some(3));
    assert!(!scroll_confirm_close(&mut state, 1));

    assert!(scroll_confirm_close(&mut state, -2));
    assert_eq!(confirm_scroll(&state), Some(1));
    assert!(scroll_confirm_close(&mut state, -10));
    assert_eq!(confirm_scroll(&state), Some(0));
    assert!(!scroll_confirm_close(&mut state, -1));

    assert!(set_confirm_close_scroll(&mut state, 99));
    assert_eq!(confirm_scroll(&state), Some(3));
    assert!(!set_confirm_close_scroll(&mut state, 3));
    assert!(set_confirm_close_scroll(&mut state, 0));
    assert_eq!(confirm_scroll(&state), Some(0));

    // 非 ConfirmClose 浮层不做任何滚动。
    state.overlay = None;
    assert!(!scroll_confirm_close(&mut state, 1));
    assert!(!set_confirm_close_scroll(&mut state, 1));
}

#[test]
fn confirm_close_up_down_keys_scroll_list_when_overflowing() {
    let mut state = AppState::demo();
    apply_workspace_history(&mut state, missing_entries(7, "confirm-keys"));
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(
        OverlayTarget::MissingWorkspaces,
    )));

    apply_overlay_key(&mut state, OverlayKey::Down);
    assert_eq!(confirm_scroll(&state), Some(1));
    apply_overlay_key(&mut state, OverlayKey::Down);
    assert_eq!(confirm_scroll(&state), Some(2));
    apply_overlay_key(&mut state, OverlayKey::Down);
    assert_eq!(confirm_scroll(&state), Some(2), "Down 到达底部后不再前进");
    apply_overlay_key(&mut state, OverlayKey::Up);
    assert_eq!(confirm_scroll(&state), Some(1));
    assert!(
        matches!(state.overlay, Some(Overlay::ConfirmClose(_))),
        "Up/Down 不关闭删除确认浮层"
    );

    // 列表不超过 5 行时保持偏移 0。
    state.workspace_history.entries.truncate(3);
    apply_overlay_key(&mut state, OverlayKey::Down);
    assert_eq!(confirm_scroll(&state), Some(0));
}

#[test]
fn delete_missing_ignores_entries_changed_after_confirmation_snapshot() {
    let mut state = AppState::demo();
    let base = std::env::temp_dir().join(format!("lx-missing-toctou-{}", std::process::id()));
    let missing_a = base.join("a-missing-at-open");
    let existing_b = base.join("b-existing-at-open");
    std::fs::create_dir_all(&existing_b).expect("create existing dir");

    apply_workspace_history(
        &mut state,
        vec![
            plain_entry("GoneA", missing_a.clone()),
            plain_entry("KeptB", existing_b.clone()),
        ],
    );
    assert!(!state.workspace_history.entries[0].exists);
    assert!(state.workspace_history.entries[1].exists);

    // 弹窗打开后磁盘变化：B 被移除但快照仍标记存在，不得借机扩大删除集合。
    std::fs::remove_dir_all(&existing_b).expect("remove existing dir");
    state.effects.clear();

    assert_eq!(delete_missing_workspace_history_entries(&mut state), 1);
    let remaining: Vec<PathBuf> = state
        .workspace_history
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    assert_eq!(
        remaining,
        vec![existing_b.clone()],
        "只删除确认快照中已失效的条目"
    );
    let deleted: Vec<PathBuf> = state
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::DeleteWorkspaceHistory { path } => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(deleted, vec![missing_a]);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn delete_missing_skips_entries_restored_after_confirmation_snapshot() {
    let mut state = AppState::demo();
    let base = std::env::temp_dir().join(format!("lx-missing-restored-{}", std::process::id()));
    let restored = base.join("restored");
    let still_missing = base.join("still-missing");
    std::fs::create_dir_all(&base).expect("create base dir");

    apply_workspace_history(
        &mut state,
        vec![
            plain_entry("Restored", restored.clone()),
            plain_entry("Gone", still_missing.clone()),
        ],
    );
    assert!(!state.workspace_history.entries[0].exists);

    // 弹窗打开后 A 被重建：确认时必须跳过已恢复的目录条目。
    std::fs::create_dir_all(&restored).expect("restore dir");
    state.effects.clear();

    assert_eq!(delete_missing_workspace_history_entries(&mut state), 1);
    let remaining: Vec<PathBuf> = state
        .workspace_history
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    assert_eq!(remaining, vec![restored.clone()], "已恢复的目录不得被删除");
    assert!(
        state.workspace_history.entries[0].exists,
        "已恢复目录的存在性需要同步回条目"
    );
    let deleted: Vec<PathBuf> = state
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::DeleteWorkspaceHistory { path } => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(deleted, vec![still_missing]);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn target_delete_entries_missing_workspaces_uses_branch_label_for_grouped_child() {
    let mut state = AppState::demo();
    let base = std::env::temp_dir().join(format!("lx-target-missing-label-{}", std::process::id()));
    let repo = base.join("repo");
    let child = repo.join("wt-feature");
    apply_workspace_history(
        &mut state,
        vec![
            git_entry("repo", repo.clone(), git_meta(&repo, &repo, false)),
            git_entry("wt-feature", child.clone(), git_meta(&repo, &child, true)),
        ],
    );

    assert_eq!(
        state
            .workspace_history
            .target_delete_entries(&OverlayTarget::MissingWorkspaces),
        vec![
            ("repo".to_string(), repo.clone()),
            ("feature/x".to_string(), child.clone()),
        ],
        "未手动改名的分组子项必须显示分支标签"
    );

    let _ = std::fs::remove_dir_all(&base);
}
