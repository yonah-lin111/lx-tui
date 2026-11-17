//! UI 测试共用夹具：工作区 git 元数据与带元数据的工作区构造。

use std::path::PathBuf;

use crate::app::state::{AppState, Workspace, WorkspaceGit};
use crate::config::Config;

/// 桌面三栏测试配置：固定 80 列阈值，避免默认 160 列把 100/120 宽度的
/// 桌面几何测试变成紧凑布局。紧凑行为由专门测试覆盖。
pub(crate) fn desktop_config() -> Config {
    Config {
        narrow_width: 80,
        ..Config::default()
    }
}

/// 构造工作区 git 元数据。
pub(crate) fn git_info(
    repo_root: &str,
    checkout: &str,
    linked: bool,
    branch: Option<&str>,
) -> WorkspaceGit {
    WorkspaceGit {
        repo_root: PathBuf::from(repo_root),
        checkout_path: PathBuf::from(checkout),
        is_linked: linked,
        branch: branch.map(str::to_string),
        main_branch: branch.map(str::to_string),
    }
}

/// 追加一个带 git 元数据的自动命名工作区；返回新工作区索引。
pub(crate) fn push_git_workspace(
    state: &mut AppState,
    name: &str,
    cwd: &str,
    git: WorkspaceGit,
) -> usize {
    let mut workspace = Workspace::single_terminal(name.to_string(), Some(PathBuf::from(cwd)));
    workspace.git = Some(git);
    state.workspaces.push(workspace);
    state.workspaces.len() - 1
}
