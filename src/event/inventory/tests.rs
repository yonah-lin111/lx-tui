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
