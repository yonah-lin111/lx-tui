//! 自定义命令重载、作用域根确定与命令视图转换。

use std::path::PathBuf;

use crate::app::state::AppState;
use crate::settings::commands::{self, CommandKind, CommandRecord, CommandScope, MdScope};

/// 重载自定义命令：User 与 Project 合并后注入 prompt；命令集变化时返回 true。
///
/// 每个显示工作区变化、设置页写入与仓库根变化后调用；`set_custom_commands`
/// 内部按内容比较，未变化时面板保持原样。
pub(crate) fn reload_custom_commands(state: &mut AppState, force: bool) -> bool {
    if !force {
        return false;
    }
    let (items, _) = custom_commands_for(state);
    let changed = items != state.prompt.custom_commands().to_vec();
    state.prompt.set_custom_commands(items);
    changed
}

/// 依据显示中 prompt 的工作区装载有效自定义命令与设置页视图快照。
pub(crate) fn custom_commands_for(
    state: &AppState,
) -> (
    Vec<crate::app::markdown::SlashItem>,
    Vec<crate::settings::service::CommandView>,
) {
    let user_root = match crate::config::user_data_root() {
        Ok(root) => root,
        Err(_) => return (Vec::new(), Vec::new()),
    };
    let (user, _user_errors) = commands::list(CommandScope::User, &user_root);
    let project_root = project_command_root(state);
    let (project, _project_errors) = match project_root.as_deref() {
        Some(root) => commands::list(CommandScope::Project, root),
        None => (Vec::new(), Vec::new()),
    };
    let mut views: Vec<crate::settings::service::CommandView> = Vec::new();
    let mut items = Vec::new();
    for record in &user {
        views.push(crate::settings::service::CommandView::of(
            record,
            CommandScope::User,
        ));
    }
    for record in &project {
        views.push(crate::settings::service::CommandView::of(
            record,
            CommandScope::Project,
        ));
    }
    for record in &user {
        items.push(slash_item_of(
            record,
            crate::app::markdown::CommandSource::User,
        ));
    }
    for record in &project {
        items.push(slash_item_of(
            record,
            crate::app::markdown::CommandSource::Project,
        ));
    }
    (items, views)
}

/// Project 作用域根：git 仓库取主 checkout；新工作区元数据未到达（cwd 已就绪）
/// 时用工作区路径；完全无路径时 None（不创建无关目录）。
fn project_command_root(state: &AppState) -> Option<PathBuf> {
    let workspace = state.prompt_owner();
    if let Some(git) = workspace.git.as_ref() {
        return Some(git.repo_root.clone());
    }
    workspace.cwd.clone().filter(|cwd| cwd.is_dir())
}

/// 命令记录 → 斜杠候选；`source` 由调用方按作用域给定。
fn slash_item_of(
    record: &CommandRecord,
    source: crate::app::markdown::CommandSource,
) -> crate::app::markdown::SlashItem {
    use crate::app::markdown::{SlashItem, SlashScope};
    match record.kind {
        CommandKind::AgentMd => SlashItem::custom_markdown(
            &record.name,
            source,
            match record.scope {
                MdScope::Global => SlashScope::Global,
                MdScope::Template => SlashScope::Template,
            },
            &record.body,
            &record.description,
        ),
        CommandKind::AgentBlock => match record.block {
            crate::settings::commands::BlockType::Template => SlashItem::custom_block(
                &record.name,
                source,
                record.title.as_deref(),
                &record.body,
                &record.description,
            ),
            crate::settings::commands::BlockType::Supple => SlashItem::custom_supple(
                &record.name,
                source,
                record.title.as_deref(),
                &record.body,
                &record.description,
            ),
        },
    }
}

/// 设置页命令快照（User 与 Project 合并后的视图）。
pub(crate) fn command_views(state: &AppState) -> Vec<crate::settings::service::CommandView> {
    custom_commands_for(state).1
}

/// 同步显示中工作区的 git 仓库根到 prompt（影响 `/gitWorkTree` 与绑定解析）；
/// 返回是否发生变化。
pub(crate) fn sync_prompt_repo(state: &mut AppState) -> bool {
    let repo = state
        .prompt_owner()
        .git
        .as_ref()
        .map(|git| git.repo_root.clone());
    state.prompt.set_worktree_repo(repo)
}

/// 启动时为已有工作区登记一次 git 元数据查询。
pub(crate) fn seed_git_requests(state: &mut AppState) {
    let cwds: Vec<PathBuf> = state
        .workspaces
        .iter()
        .filter_map(|workspace| workspace.cwd.clone())
        .collect();
    for cwd in cwds {
        crate::app::update::request_git_refresh(state, &cwd);
    }
}
