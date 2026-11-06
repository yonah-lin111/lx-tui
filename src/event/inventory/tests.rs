use super::*;
use crate::app::state::WorkspaceGit;

#[test]
fn skill_roots_tracks_git_refresh_and_multiple_workspaces() {
    let mut state = AppState::demo();
    state.workspaces[0].cwd = Some(PathBuf::from("/repo/.worktrees/wt"));
    state.workspaces[0].git = None;
    assert_eq!(
        skill_roots(&state),
        vec![Some(PathBuf::from("/repo/.worktrees/wt"))]
    );

    // 模拟 GitRefreshed 将 worktree 归位到主仓库根：skill_roots 必须变化以触发重扫。
    state.workspaces[0].git = Some(WorkspaceGit {
        repo_root: PathBuf::from("/repo"),
        checkout_path: PathBuf::from("/repo/.worktrees/wt"),
        branch: Some("feat".into()),
        main_branch: Some("dev".into()),
        is_linked: true,
    });
    assert_eq!(skill_roots(&state), vec![Some(PathBuf::from("/repo"))]);

    // 再打开第二个不同仓库的工作区（即使 prompt 钉在第一个工作区）：两个根都必须被收集。
    state.prompt_pinned = Some(0);
    let mut other = AppState::demo().workspaces.remove(0);
    other.cwd = Some(PathBuf::from("/other-repo"));
    other.git = None;
    state.workspaces.push(other);
    assert_eq!(
        skill_roots(&state),
        vec![
            Some(PathBuf::from("/repo")),
            Some(PathBuf::from("/other-repo"))
        ]
    );
}

#[test]
fn update_mcp_status_computes_live_ready_and_error_counts_and_clears_removed_errors() {
    let mut state = AppState::demo();
    let root = PathBuf::from("/repo");
    state.mcp_inventory.insert(
        root.clone(),
        StartupCount {
            total: 3,
            ready: 0,
            errors: 0,
            loading: true,
        },
    );
    let mut statuses = BTreeMap::new();
    statuses.insert(
        (root.clone(), "alpha".to_string()),
        McpStatusView {
            project: root.clone(),
            server: "alpha".to_string(),
            status: "ready".to_string(),
            error: None,
        },
    );
    update_mcp_status(&mut state, &root, &statuses);
    let count = state.mcp_inventory[&root];
    assert_eq!(count.ready, 1);
    assert_eq!(count.errors, 0);
    assert!(count.loading, "其余两个服务尚未结算时 loading 仍为 true");

    statuses.insert(
        (root.clone(), "beta".to_string()),
        McpStatusView {
            project: root.clone(),
            server: "beta".to_string(),
            status: "failed".to_string(),
            error: Some("boom".to_string()),
        },
    );
    statuses.insert(
        (root.clone(), "gamma".to_string()),
        McpStatusView {
            project: root.clone(),
            server: "gamma".to_string(),
            status: "ready".to_string(),
            error: None,
        },
    );
    update_mcp_status(&mut state, &root, &statuses);
    let count = state.mcp_inventory[&root];
    assert_eq!(count.ready, 2);
    assert_eq!(count.errors, 1);
    assert!(!count.loading, "3/3 已结算后 loading 结束");

    // 模拟用户在设置页禁用失败的 beta：剔除状态并更新 total=2 后，错误计数应立即归零。
    statuses.remove(&(root.clone(), "beta".to_string()));
    state.mcp_inventory.entry(root.clone()).or_default().total = 2;
    update_mcp_status(&mut state, &root, &statuses);
    let count = state.mcp_inventory[&root];
    assert_eq!(count.total, 2);
    assert_eq!(count.ready, 2);
    assert_eq!(count.errors, 0);
    assert!(!count.loading);
}

#[test]
fn refresh_agents_md_updates_count_when_workspace_becomes_trusted() {
    let dir = std::env::temp_dir().join(format!(
        "lx-inventory-agents-md-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("AGENTS.md"), "# Project rules\n").expect("write AGENTS.md");
    let db_path = dir.join("lx.db");
    let database = crate::db::Database::open(&db_path).expect("open db");
    let mut persistence = Persistence::with_database(database);

    let mut state = AppState::demo();
    state.workspaces[0].cwd = Some(dir.clone());
    state.workspaces[0].git = None;

    refresh_agents_md(&mut state, &persistence);
    let untrusted_count = state.agents_md_inventory[&Some(dir.clone())];

    state.effects.push(crate::app::effects::Effect::GrantTrust {
        path: dir.clone(),
        name: "repo".to_string(),
    });
    assert!(persistence.pump(&mut state));
    refresh_agents_md(&mut state, &persistence);
    let trusted_count = state.agents_md_inventory[&Some(dir.clone())];
    assert_eq!(
        trusted_count,
        untrusted_count + 1,
        "授予信任后项目级非空 AGENTS.md 应计入"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn refresh_agents_md_isolates_linked_worktree_and_subdirectory_counts() {
    let base = std::env::temp_dir().join(format!(
        "lx-inventory-agents-worktree-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let repo_root = base.join("repo");
    let pkg_dir = repo_root.join("packages").join("pkg-a");
    let worktree_dir = base.join("wt-1");
    std::fs::create_dir_all(&pkg_dir).expect("pkg dir");
    std::fs::create_dir_all(&worktree_dir).expect("wt dir");
    std::fs::create_dir_all(repo_root.join(".git")).expect("repo .git");
    // linked worktree 的 .git 是文件，指示 collect_search_dirs 在 worktree 根止步。
    std::fs::write(
        worktree_dir.join(".git"),
        "gitdir: ../repo/.git/worktrees/wt-1\n",
    )
    .expect("wt .git");

    std::fs::write(repo_root.join("AGENTS.md"), "# Root rules\n").expect("root AGENTS.md");
    std::fs::write(pkg_dir.join("AGENTS.md"), "# Package rules\n").expect("pkg AGENTS.md");
    // worktree_dir 不写 AGENTS.md，验证它不会误把主 checkout 的 AGENTS.md 算进来。

    let db_path = base.join("lx.db");
    let database = crate::db::Database::open(&db_path).expect("open db");
    let mut persistence = Persistence::with_database(database);

    let mut state = AppState::demo();
    state.workspaces[0].cwd = Some(repo_root.clone());
    state.workspaces[0].git = Some(WorkspaceGit {
        repo_root: repo_root.clone(),
        checkout_path: repo_root.clone(),
        branch: Some("dev".into()),
        main_branch: Some("dev".into()),
        is_linked: false,
    });

    let mut sub_ws = AppState::demo().workspaces.remove(0);
    sub_ws.cwd = Some(pkg_dir.clone());
    sub_ws.git = Some(WorkspaceGit {
        repo_root: repo_root.clone(),
        checkout_path: repo_root.clone(),
        branch: Some("dev".into()),
        main_branch: Some("dev".into()),
        is_linked: false,
    });
    state.workspaces.push(sub_ws);

    let mut wt_ws = AppState::demo().workspaces.remove(0);
    wt_ws.cwd = Some(worktree_dir.clone());
    wt_ws.git = Some(WorkspaceGit {
        repo_root: repo_root.clone(),
        checkout_path: worktree_dir.clone(),
        branch: Some("feat".into()),
        main_branch: Some("dev".into()),
        is_linked: true,
    });
    state.workspaces.push(wt_ws);

    // 信任主仓库根：同仓库子目录与 linked worktree 均应视为已信任。
    state.effects.push(crate::app::effects::Effect::GrantTrust {
        path: repo_root.clone(),
        name: "repo".to_string(),
    });
    let _ = persistence.pump(&mut state);

    refresh_agents_md(&mut state, &persistence);
    assert!(state.sync_lx_inventory());

    let root_count = state.agents_md_inventory[&Some(repo_root.clone())];
    let pkg_count = state.agents_md_inventory[&Some(pkg_dir.clone())];
    let wt_count = state.agents_md_inventory[&Some(worktree_dir.clone())];

    assert_eq!(
        pkg_count,
        root_count + 1,
        "子目录工作区应累计 repo_root + pkg_dir 两层 AGENTS.md"
    );
    assert_eq!(
        wt_count + 1,
        root_count,
        "linked worktree 无自身 AGENTS.md 时不应误计主 checkout 的 AGENTS.md"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn drain_mcp_events_updates_open_mcp_panel_and_suppresses_toast_on_disabled_pane() {
    use crate::app::state::{
        Direction, McpCatalogEntry, McpRuntimeState, McpServerState, SlashCommand, SlashPanel,
    };

    let mut state = AppState::demo();
    let root = PathBuf::from("/repo");
    state.workspaces[0].cwd = Some(root.clone());
    state.workspaces[0].git = None;
    state.mcp_catalog.insert(
        root.clone(),
        vec![
            McpCatalogEntry {
                name: "alpha".into(),
                summary: "stdio: alpha".into(),
                config_enabled: true,
                runtime: McpRuntimeState::Connecting,
            },
            McpCatalogEntry {
                name: "beta".into(),
                summary: "stdio: beta".into(),
                config_enabled: true,
                runtime: McpRuntimeState::Connecting,
            },
        ],
    );

    let pane1 = state.active_tab().layout.focus();
    let pane2 = state
        .active_tab_mut()
        .split_pane(pane1, Direction::Horizontal)
        .expect("split pane");

    // Disable "beta" in pane1, keep it enabled in pane2; open `/mcp` panel in pane1.
    state
        .pane_mut_anywhere(pane1)
        .expect("pane1")
        .lx
        .set_mcp_server_disabled("beta", true);
    assert!(update::apply_slash_command(
        &mut state,
        pane1,
        SlashCommand::Mcp
    ));

    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let _ = sender.send(McpEvent::Status {
        project: root.clone(),
        server: "alpha".into(),
        status: ServerStatus::Ready { tools: 5 },
    });
    let _ = sender.send(McpEvent::Status {
        project: root,
        server: "beta".into(),
        status: ServerStatus::Failed {
            error: "timeout".into(),
        },
    });

    let mut statuses = BTreeMap::new();
    let mut toasted = HashSet::new();
    assert!(drain_mcp_events(
        &mut receiver,
        &mut state,
        None,
        &mut statuses,
        &mut toasted
    ));

    // Open `/mcp` panel in pane1 updates "alpha" to Connected { tools: 5 } and keeps "beta" DisabledInPane.
    let p1 = &state.pane_anywhere(pane1).expect("pane1").lx;
    let Some(SlashPanel::Mcp(panel)) = &p1.slash else {
        panic!("mcp panel open");
    };
    assert_eq!(
        panel.entries[0].state,
        McpServerState::Connected { tools: 5 }
    );
    assert_eq!(panel.entries[1].state, McpServerState::DisabledInPane);

    // Failure toast for "beta" is suppressed on pane1 (where "beta" is disabled) and shown on pane2.
    assert!(p1.toast.is_none(), "disabled pane must not receive toast");
    assert!(
        state
            .pane_anywhere(pane2)
            .expect("pane2")
            .lx
            .toast
            .is_some(),
        "enabled pane receives failure toast"
    );
}
