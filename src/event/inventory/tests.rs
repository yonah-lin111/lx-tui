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
